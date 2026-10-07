//! 通道B sidecar 进程管理（frida_bridge.py，JSON-RPC over stdio，换行分帧）。
//! 三通道共用 FridaBackend trait（文档02/09）；M1 实现通道B，通道C（CLI 解析）在
//! sidecar 不可用时给出明确降级错误，不做静默兜底。

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{broadcast, mpsc, oneshot, Mutex, RwLock};
use tokio::time::Duration;

/// 注入 agent 源码（构建期内嵌，运行期零外部文件依赖）
pub const CORE_AGENT_JS: &str = include_str!("../../../agent/dist/core.js");

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", content = "params", rename_all = "snake_case")]
pub enum FridaEvent {
    /// sidecar 就绪
    Ready { frida: String },
    /// agent send() 的结构化消息（文档02：唯一可信数据通道）
    Message {
        script_id: u64,
        kind: String,
        payload: Option<Value>,
        description: Option<String>,
        stack: Option<String>,
        data_b64: Option<String>,
    },
    /// 会话分离（进程退出/崩溃/连接断开）
    Detached {
        session_id: u64,
        reason: String,
        crash: Option<Value>,
    },
    /// 设备失联
    DeviceLost { device: String },
    SpawnAdded {
        identifier: Option<String>,
        pid: Option<u32>,
    },
    /// 请求在 sidecar 侧 25s 结构化超时被弃管（宿主已收到 timeout 错误）；
    /// 底层调用若最终完成会补发 OpLate 并由 sidecar 回滚创建型副作用（批次⑩）
    OpAbandoned { req_id: u64, method: String },
    /// 被弃管的请求最终完成（ok=false 表示以异常收场）；宿主只留痕，不恢复状态
    OpLate {
        req_id: u64,
        method: String,
        ok: bool,
        result: Option<Value>,
        error: Option<String>,
    },
}

struct Pending {
    map: Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>,
    next_id: AtomicU64,
}

impl Pending {
    /// 进程死亡时把全部在途请求打成错误（drain 幂等，多处调用安全）
    async fn fail_all(&self, reason: &str) {
        let mut map = self.map.lock().await;
        for (_, tx) in map.drain() {
            let _ = tx.send(Err(reason.to_string()));
        }
    }
}

struct SidecarInner {
    stdin: Mutex<mpsc::Sender<String>>,
    pending: Arc<Pending>,
    events_tx: broadcast::Sender<FridaEvent>,
    alive: Arc<AtomicBool>,
}

/// 通道B 句柄：懒启动；sidecar 意外退出后，下次调用自动重启（幂等，S-03）。
#[derive(Clone)]
pub struct FridaChannelB {
    inner: Arc<RwLock<Option<Arc<SidecarInner>>>>,
    /// spawn 单飞锁：并发首调（如诊断巡检撞上 UI 刷新）此前会双双 spawn，
    /// 后者整体替换 inner 使前者被 kill_on_drop 杀死 → "sidecar 已退出"
    spawn_lock: Arc<Mutex<()>>,
    launch: crate::paths::SidecarLaunch,
}

impl FridaChannelB {
    pub fn new(launch: crate::paths::SidecarLaunch) -> Self {
        Self {
            inner: Arc::new(RwLock::new(None)),
            spawn_lock: Arc::new(Mutex::new(())),
            launch,
        }
    }

    pub async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        let s = self.ensure().await?;
        s.call(method, params).await
    }

    pub async fn subscribe(&self) -> Result<broadcast::Receiver<FridaEvent>, String> {
        let s = self.ensure().await?;
        Ok(s.subscribe())
    }

    /// 只订阅「当前已存活」的 sidecar，不主动拉起进程（forward_events 的重订阅用）。
    /// 订阅前后各查一次 alive：把「订阅瞬间进程死亡」的窗口缩到最小。
    pub async fn subscribe_existing(&self) -> Result<broadcast::Receiver<FridaEvent>, String> {
        let s = self
            .inner
            .read()
            .await
            .as_ref()
            .cloned()
            .ok_or("sidecar 未启动")?;
        if !s.alive.load(Ordering::SeqCst) {
            return Err("sidecar 已退出".into());
        }
        let rx = s.subscribe();
        if !s.alive.load(Ordering::SeqCst) {
            return Err("sidecar 已退出".into());
        }
        Ok(rx)
    }

    #[allow(dead_code)] // M2 通道降级 UI 启用
    pub fn channel_tag(&self) -> &'static str {
        "B"
    }

    async fn ensure(&self) -> Result<Arc<SidecarInner>, String> {
        {
            let s = self.inner.read().await;
            if let Some(s) = s.as_ref() {
                if s.alive.load(Ordering::SeqCst) {
                    return Ok(s.clone());
                }
            }
        }
        // 单飞：拿到 spawn 锁后二次确认（别的调用可能已在我们排队时拉起）
        let _guard = self.spawn_lock.lock().await;
        {
            let s = self.inner.read().await;
            if let Some(s) = s.as_ref() {
                if s.alive.load(Ordering::SeqCst) {
                    return Ok(s.clone());
                }
            }
        }
        let s = spawn_sidecar(&self.launch).await?;
        *self.inner.write().await = Some(s.clone());
        tracing::info!("[通道B] sidecar 已启动（{}）", self.launch.label);
        Ok(s)
    }
}

async fn spawn_sidecar(launch: &crate::paths::SidecarLaunch) -> Result<Arc<SidecarInner>, String> {
    // python 源码模式需校验脚本存在；exe 模式自包含无需校验
    let python_mode = launch.program.ends_with(".py");
    if python_mode {
        let bridge = crate::paths::sidecar_bridge_path();
        if !bridge.is_file() {
            return Err(format!("sidecar 脚本缺失：{}", bridge.display()));
        }
    }
    let mut cmd = Command::new(&launch.program);
    cmd.args(&launch.args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    // sidecar 的 python（exe 模式为 PyInstaller 内嵌解释器）stdout/stdin 编码在
    // Windows 管道上默认随 locale（GBK）：应用名等非 ASCII 一写就变 GBK 字节，
    // 宿主按 UTF-8 读必炸。源头强制 UTF-8。
    cmd.env("PYTHONUTF8", "1").env("PYTHONIOENCODING", "utf-8");
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("启动 sidecar 失败（{}）：{e}", launch.label))?;
    let stdin = child.stdin.take().ok_or("sidecar stdin 不可用")?;
    let stdout = child.stdout.take().ok_or("sidecar stdout 不可用")?;
    let stderr = child.stderr.take().ok_or("sidecar stderr 不可用")?;

    let (events_tx, _) = broadcast::channel(512);
    let pending: Arc<Pending> = Arc::new(Pending {
        map: Mutex::new(HashMap::new()),
        next_id: AtomicU64::new(1),
    });
    let (tx_stdin, mut rx_stdin) = mpsc::channel::<String>(64);
    let alive = Arc::new(AtomicBool::new(true));

    // stdin 写泵
    tokio::spawn(async move {
        let mut stdin = stdin;
        while let Some(line) = rx_stdin.recv().await {
            if stdin.write_all(line.as_bytes()).await.is_err() {
                break;
            }
            if stdin.write_all(b"\n").await.is_err() {
                break;
            }
            if stdin.flush().await.is_err() {
                break;
            }
        }
    });

    // stdout 读泵：分帧分发 response / event。sidecar 存活的唯一可信信号 = 这条管道活着：
    // alive 只在此处置 false；PyInstaller onefile 的 bootloader 可能先于子进程退出，
    // wait() 返回 ≠ 管道死（此前在此误判 "sidecar 已退出"，孤儿子进程变僵尸）。
    {
        let pending = pending.clone();
        let events_tx = events_tx.clone();
        let alive = alive.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();
            loop {
                let line = match reader.next_line().await {
                    Ok(Some(l)) => l,
                    Ok(None) => break,
                    // 单行解码/IO 错误不终止事件管线（取证数据 > 完美主义）：记日志丢行
                    Err(e) => {
                        tracing::warn!("[通道B] stdout 读行错误（丢行继续）：{e}");
                        continue;
                    }
                };
                let v: Value = match serde_json::from_str(&line) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                match v.get("type").and_then(|t| t.as_str()) {
                    Some("response") => {
                        let id = v.get("id").and_then(|i| i.as_u64()).unwrap_or(0);
                        let result = if let Some(e) = v.get("error").filter(|e| !e.is_null()) {
                            Err(e.as_str().unwrap_or("sidecar error").to_string())
                        } else {
                            Ok(v.get("result").cloned().unwrap_or(Value::Null))
                        };
                        if let Some(tx) = pending.map.lock().await.remove(&id) {
                            let _ = tx.send(result);
                        }
                    }
                    Some("event") => {
                        if let Ok(ev) = serde_json::from_value::<FridaEvent>(v) {
                            let _ = events_tx.send(ev);
                        }
                    }
                    _ => {}
                }
            }
            alive.store(false, Ordering::SeqCst);
            // 进程死亡兜底广播（会话管理器据此把状态打到失败并留证据）
            let _ = events_tx.send(FridaEvent::Detached {
                session_id: 0,
                reason: "connectionTerminated(sidecar exited)".into(),
                crash: None,
            });
            // 在途请求立即报错返回，不让调用方干等 30s 超时
            pending.fail_all("sidecar 已退出").await;
        });
    }

    // stderr → 应用日志（sidecar 的堆栈追踪都在这里）
    tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            tracing::debug!("[sidecar] {line}");
        }
    });

    // 退出看护：wait() 只归收子进程句柄（僵尸回收），不宣告死亡
    let child = Arc::new(Mutex::new(Some(child)));
    {
        let child = child.clone();
        tokio::spawn(async move {
            let mut slot = child.lock().await;
            if let Some(mut c) = slot.take() {
                let _ = c.wait().await;
            }
            // 只归收僵尸、不宣告死亡：alive 的唯一权威是 stdout 读泵（管道 EOF）。
            // PyInstaller onefile 的 bootloader 可能先退，wait() 返回时子进程仍在服务。
        });
    }

    Ok(Arc::new(SidecarInner {
        stdin: Mutex::new(tx_stdin),
        pending,
        events_tx,
        alive,
    }))
}

impl SidecarInner {
    pub async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        if !self.alive.load(Ordering::SeqCst) {
            return Err("sidecar 已退出（将在下次调用自动重启）".into());
        }
        let id = self.pending.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.map.lock().await.insert(id, tx);
        let req = json!({"id": id, "method": method, "params": params});
        if self.stdin.lock().await.send(req.to_string()).await.is_err() {
            // stdin 已关闭：立即清掉在途 entry（否则 entry 泄漏到 fail_all 才收走）
            self.pending.map.lock().await.remove(&id);
            return Err("sidecar stdin 已关闭".into());
        }
        match tokio::time::timeout(Duration::from_secs(30), rx).await {
            Ok(Ok(res)) => res,
            Ok(Err(_)) => Err("sidecar 响应通道关闭".into()),
            Err(_) => {
                self.pending.map.lock().await.remove(&id);
                Err(format!("sidecar 调用超时：{method}（30s）"))
            }
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<FridaEvent> {
        self.events_tx.subscribe()
    }
}

/// 附加 + 加载 core agent 的完整链（M1；会话状态机在 services/session.rs 之上组装）
pub async fn attach_and_load_core(
    ch: &FridaChannelB,
    host: &str,
    port: u16,
    target: Value,
) -> Result<(u64, u64), String> {
    let conn = ch
        .call("remote_connect", json!({"host": host, "port": port}))
        .await?;
    let device = conn
        .get("key")
        .and_then(|k| k.as_str())
        .ok_or("remote_connect 未返回 device key")?
        .to_string();
    let att = ch.call("attach", json!({"device": device, "target": target})).await?;
    let session_id = att
        .get("session_id")
        .and_then(|v| v.as_u64())
        .ok_or("attach 未返回 session_id")?;
    let script = ch
        .call(
            "create_script",
            json!({"session_id": session_id, "source": CORE_AGENT_JS, "name": "lovelyfrida-core"}),
        )
        .await?;
    let script_id = script
        .get("script_id")
        .and_then(|v| v.as_u64())
        .ok_or("create_script 未返回 script_id")?;
    ch.call("load_script", json!({"script_id": script_id})).await?;
    Ok((session_id, script_id))
}
