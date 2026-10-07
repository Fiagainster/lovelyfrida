//! 通道C：frida CLI 兜底（文档02 三通道设计；文档10 P3-4）。
//!
//! 能力边界（诚实降级，不做静默假装）：
//! - **可用**：附加 + 加载 core agent + 全量观测——agent 的 send() 结构化消息经
//!   `@@LF@@` 前缀 shim 从 CLI stdout 回流，复用通道B 的事件管线（trace/时间轴/审计）。
//! - **不可用**：交互式 rpc_call（探针填表/探索器/REPL）——调用方明确报错，引导装通道B。
//!
//! shim 原理：core.js 会猴子补丁 console 并经 send() 分级上报；本 shim 在 core.js
//! **之前**捕获原始 console.log 并接管 send()，因此 shim 内部打日志走原始 console
//! （无递归），core.js 补丁后的 console 输出走 send → shim → 原始 console 打前缀行。
use crate::backends::frida::{FridaEvent, CORE_AGENT_JS};
use serde_json::json;
use std::sync::Arc;
use tauri::{Emitter, Manager};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{broadcast, Mutex};

pub const SEND_PREFIX: &str = "@@LF@@";
/// CLI 模式的固定虚拟 id（通道B 的句柄 id 语义在 C 下不存在）
pub const VIRTUAL_SCRIPT_ID: u64 = 1;

struct CInner {
    child: tokio::process::Child,
}

/// 通道C 句柄：同一时刻一个 CLI 会话（与单会话模型一致）。
pub struct FridaChannelC {
    inner: Arc<Mutex<Option<CInner>>>,
    events_tx: broadcast::Sender<FridaEvent>,
}

impl FridaChannelC {
    pub fn new() -> Self {
        let (events_tx, _) = broadcast::channel(512);
        Self {
            inner: Arc::new(Mutex::new(None)),
            events_tx,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<FridaEvent> {
        self.events_tx.subscribe()
    }

    /// frida CLI 是否可用（PATH 上能执行 `frida --version`）
    pub async fn available() -> bool {
        matches!(
            crate::backends::adb::run_raw(std::path::Path::new("frida"), &["--version"], std::time::Duration::from_secs(10)).await,
            Ok(o) if !o.timed_out && o.code == Some(0)
        )
    }

    /// 生成 send-shim 包装脚本（workspace 唯一可写区），返回路径
    async fn wrapped_agent_path() -> Result<std::path::PathBuf, String> {
        let cfg = crate::config::get();
        let dir = crate::paths::workspace_root(&cfg);
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| e.to_string())?;
        let p = dir.join("_agent_c.js");
        let shim = format!(
            r#"// LovelyFrida 通道C send-shim（自动生成，勿手改）
// 必须在 core.js 之前执行：捕获原始 console.log，接管 send() 走前缀行回流
(function () {{
  var __origLog = console.log;
  var __origSend = globalThis.send;
  globalThis.send = function (msg) {{
    try {{ __origLog.call(console, "{prefix}" + JSON.stringify(msg)); }} catch (e) {{}}
    try {{ if (__origSend) __origSend.apply(this, arguments); }} catch (e) {{}}
  }};
}})();
"#,
            prefix = SEND_PREFIX,
        );
        tokio::fs::write(&p, format!("{shim}{CORE_AGENT_JS}"))
            .await
            .map_err(|e| format!("写通道C 包装脚本失败：{e}"))?;
        Ok(p)
    }

    /// 附加目标并加载 agent（target：pid 数字串或进程名，交 CLI 解析）。
    /// spawn 后立即建立 stdout/stderr 泵（事件进 broadcast + 前端 + trace 管线）。
    pub async fn attach_and_load(
        &self,
        app: tauri::AppHandle,
        host: &str,
        host_port: u16,
        target: &str,
    ) -> Result<(), String> {
        {
            let mut guard = self.inner.lock().await;
            if let Some(c) = guard.as_mut() {
                if matches!(c.child.try_wait(), Ok(None)) {
                    return Err("通道C 已有活动会话：先分离再附加新目标".into());
                }
            }
        }
        let script_path = Self::wrapped_agent_path().await?;
        let mut cmd = Command::new("frida");
        cmd.args([
            "-q",
            "-H",
            &format!("{host}:{host_port}"),
            "-l",
            &script_path.display().to_string(),
            target,
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000);

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("启动 frida CLI 失败（通道C，需 PATH 上有 frida）：{e}"))?;
        let stdout = child.stdout.take().ok_or("通道C stdout 不可用")?;
        let stderr = child.stderr.take().ok_or("通道C stderr 不可用")?;

        {
            let mut guard = self.inner.lock().await;
            *guard = Some(CInner { child });
        }

        // stdout 泵：@@LF@@ 前缀行 → 结构化事件（前端 + trace + broadcast）；EOF → detached
        {
            let tx = self.events_tx.clone();
            let app = app.clone();
            let inner = self.inner.clone();
            tokio::spawn(async move {
                let reader = BufReader::new(stdout);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let Some(payload) = line.strip_prefix(SEND_PREFIX) else {
                        continue;
                    };
                    let v: serde_json::Value =
                        serde_json::from_str(payload).unwrap_or(serde_json::Value::Null);
                    forward_message(&app, &v);
                    let _ = tx.send(FridaEvent::Message {
                        script_id: VIRTUAL_SCRIPT_ID,
                        kind: "send".into(),
                        payload: Some(v),
                        description: None,
                        stack: None,
                        data_b64: None,
                    });
                }
                let _ = app.emit(
                    "frida-event",
                    json!({"event": "detached", "params": {"reason": "connectionTerminated(channel-c cli exited)"}}),
                );
                let _ = tx.send(FridaEvent::Detached {
                    session_id: 0,
                    reason: "connectionTerminated(channel-c cli exited)".into(),
                    crash: None,
                });
                let mut guard = inner.lock().await;
                *guard = None;
            });
        }
        // stderr 泵：CLI/脚本报错 → 前端 + 日志（此前只进 broadcast——通道C 没有全局
        // 转发者，hello 之后没人消费，报错永远到不了 UI）
        {
            let tx = self.events_tx.clone();
            let app = app.clone();
            tokio::spawn(async move {
                let reader = BufReader::new(stderr);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tracing::warn!("[通道C] {line}");
                    let payload = json!({
                        "t": "console", "level": "error",
                        "args": [{ "k": "str", "v": line }]
                    });
                    let _ = app.emit(
                        "frida-event",
                        json!({"event": "message", "params": {"kind": "error", "payload": payload}}),
                    );
                    let _ = tx.send(FridaEvent::Message {
                        script_id: VIRTUAL_SCRIPT_ID,
                        kind: "error".into(),
                        payload: Some(payload),
                        description: None,
                        stack: None,
                        data_b64: None,
                    });
                }
            });
        }
        tracing::info!(
            "[通道C] CLI 已附加（target={target}，script={}）",
            script_path.display()
        );
        Ok(())
    }

    /// 分离：杀 CLI 子进程（stdout EOF → 泵自清）
    pub async fn detach(&self) {
        let mut guard = self.inner.lock().await;
        if let Some(c) = guard.as_mut() {
            let _ = c.child.kill().await;
            tracing::info!("[通道C] CLI 进程已终止");
        }
    }
}

/// 与 forward_events（通道B）同路的本地转发：emit 前端 + 直写 trace 管线
fn forward_message(app: &tauri::AppHandle, payload: &serde_json::Value) {
    let trace: tauri::State<'_, Arc<crate::services::trace::TraceState>> = app.state();
    crate::services::trace::on_agent_message(app, &trace, payload, VIRTUAL_SCRIPT_ID, &None);
    let _ = app.emit(
        "frida-event",
        json!({"event": "message", "params": {"kind": "send", "payload": payload}}),
    );
}

/// `frida-ps -H host:port` 进程枚举解析（通道C 兜底；返回 (pid, name)）
pub async fn enumerate_processes_cli(
    host: &str,
    host_port: u16,
) -> Result<Vec<(u32, String)>, String> {
    let out = crate::backends::adb::run_raw(
        std::path::Path::new("frida-ps"),
        &["-H", &format!("{host}:{host_port}")],
        std::time::Duration::from_secs(20),
    )
    .await
    .map_err(|e| format!("执行 frida-ps 失败（通道C）：{e}"))?;
    if out.timed_out {
        return Err("frida-ps 超时（通道C）".into());
    }
    let mut rows: Vec<(u32, String)> = Vec::new();
    for line in out.stdout.lines().skip(1) {
        // 输出形如 "  PID  Name"：首列为 pid，其余为名称
        let trimmed = line.trim_start();
        let Some((pid_str, name)) = trimmed.split_once(char::is_whitespace) else {
            continue;
        };
        if let Ok(pid) = pid_str.parse::<u32>() {
            rows.push((pid, name.trim().to_string()));
        }
    }
    if rows.is_empty() && !out.stdout.contains("PID") {
        return Err(format!(
            "frida-ps 无可解析输出（通道C）：{}",
            out.stderr.trim()
        ));
    }
    Ok(rows)
}
