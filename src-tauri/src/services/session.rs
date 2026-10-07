//! 会话管理（文档04-B）：托管「设备→frida-server→forward→attach→agent 注入」链路。
//! 状态机（文档02§五）：不回退——失败保留现场，可只重试该步。
use crate::backends::adb::AdbBackend;
use crate::backends::frida::{attach_and_load_core, FridaChannelB, FridaEvent};
use crate::config::AppConfig;
use serde::Serialize;
use serde_json::{json, Value};
use std::time::Duration;
use tauri::{Emitter, Manager, State};
use tokio::sync::Mutex;

/// 会话阶段（文档02§五 的 M1 可用子集；APP_INSTALLED/DATA_INJECTED 由 M3 回灌写入）
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionPhase {
    Idle,
    Discovering,
    DeviceReady,
    ServerUp,
    Forwarded,
    Attached,
    Running,
    Failed,
    Stopped,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionSnapshot {
    pub phase: SessionPhase,
    pub evidence: Vec<String>,
    pub device: Option<String>,
    /// forward 的主机侧端口（S-06：与设备端口不一致时由探测得出）
    pub forward_host_port: Option<u16>,
    pub target: Option<String>,
    pub session_id: Option<u64>,
    pub script_id: Option<u64>,
    pub hello: Option<Value>,
    pub channel: String,
    pub updated_at: String,
    /// cases.db sessions 行 id（P2-3 落库；None=落库失败或库不可用，不阻断分析）
    #[serde(default)]
    pub db_session_id: Option<i64>,
}

impl Default for SessionSnapshot {
    fn default() -> Self {
        Self {
            phase: SessionPhase::Idle,
            evidence: Vec::new(),
            device: None,
            forward_host_port: None,
            target: None,
            session_id: None,
            script_id: None,
            hello: None,
            channel: "B".into(),
            updated_at: chrono::Local::now().format("%H:%M:%S%.3f").to_string(),
            db_session_id: None,
        }
    }
}

/// tauri managed state
pub struct FridaState {
    pub channel: FridaChannelB,
    /// 通道C（frida CLI 兜底，文档10 P3-4）：观测级降级
    pub channel_c: std::sync::Arc<crate::backends::frida_c::FridaChannelC>,
    /// preferred_channel（auto|a|b|c）：驱动 attach 的通道选择，UI 必须明示当前通道
    pub preferred_channel: String,
    pub session: Mutex<SessionSnapshot>,
    /// attach/detach 操作护栏：双击附加此前会跑两条完整链路（两个 agent 先后加载，
    /// 仅最后一个 script_id 被 detach）；也与 graceful_shutdown 的限时清理互斥
    pub op_lock: Mutex<()>,
}

impl FridaState {
    pub fn new(launch: crate::paths::SidecarLaunch, preferred_channel: String) -> Self {
        Self {
            channel: FridaChannelB::new(launch),
            channel_c: std::sync::Arc::new(crate::backends::frida_c::FridaChannelC::new()),
            preferred_channel,
            session: Mutex::new(SessionSnapshot::default()),
            op_lock: Mutex::new(()),
        }
    }
}

/// evidence 上限：长会话多次 attach/detach 下 snapshot 线性膨胀（每次 emit 全量 clone）。
/// 滚动丢弃最旧条目——近期证据的价值高于远古证据。
const EVIDENCE_CAP: usize = 400;

pub(crate) fn push_ev(s: &mut SessionSnapshot, msg: impl Into<String>) {
    if s.evidence.len() >= EVIDENCE_CAP {
        s.evidence.remove(0);
    }
    s.evidence.push(msg.into());
}

pub(crate) fn extend_ev(s: &mut SessionSnapshot, msgs: impl IntoIterator<Item = String>) {
    for m in msgs {
        push_ev(s, m);
    }
}

/// frida-server 二进制定位：随包 bin/ 矩阵优先，工作区按需下载（C1）兜底
pub fn find_server_binary(
    cfg: &AppConfig,
    version: &str,
    abi_dir: &str,
) -> Option<std::path::PathBuf> {
    let bundled = crate::paths::frida_server_matrix_dir()
        .join(version)
        .join(format!("android-{abi_dir}"))
        .join("frida-server");
    if bundled.is_file() {
        return Some(bundled);
    }
    let ws = crate::paths::workspace_root(cfg)
        .join("frida-server")
        .join(version)
        .join(format!("android-{abi_dir}"))
        .join("frida-server");
    ws.is_file().then_some(ws)
}

fn abi_to_dirname(abi: &str) -> &str {
    match abi {
        "arm64-v8a" => "arm64",
        "armeabi-v7a" => "arm",
        "x86_64" => "x86_64",
        "x86" => "x86",
        other => other,
    }
}

fn step(name: &str, status: &str, evidence: Vec<String>) -> StepReport {
    StepReport {
        name: name.into(),
        status: status.into(),
        evidence,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct StepReport {
    pub name: String,
    pub status: String, // pass | warn | fail
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ServerStatusReport {
    pub client_version: Option<String>,
    pub client_error: Option<String>,
    pub device_serial: Option<String>,
    pub device_server_present: Option<bool>,
    pub device_server_version: Option<String>,
    pub server_running: Option<bool>,
    pub forward_established: Option<bool>,
    pub matrix: Vec<String>,
    pub port: u16,
    pub overall: String, // pass | warn | fail
}

/// frida 环境总览（版本三处一致检查的运行时版，S-01）
pub async fn server_status(cfg: &AppConfig, frida: &FridaChannelB) -> ServerStatusReport {
    let port = cfg.frida_port;

    // 并行层1：sidecar hello 与 adb 探测互不依赖（此前 7 个串行 await，
    // SessionConsole 每次刷新都是整条链跑一遍）
    let (hello_res, adb_res) = tokio::join!(
        frida.call("hello", json!({})),
        AdbBackend::detect(&cfg.adb_path, &cfg.doctor.adb_extra_paths),
    );
    let (client_version, client_error) = match hello_res {
        Ok(v) => (
            v.get("frida").and_then(|s| s.as_str()).map(String::from),
            None,
        ),
        Err(e) => (None, Some(e)),
    };
    let adb = adb_res.ok();
    let devices = match &adb {
        Some(a) => a.devices().await.unwrap_or_default(),
        None => Vec::new(),
    };
    let serial = devices
        .iter()
        .find(|d| d.state == "device")
        .map(|d| d.serial.clone());

    let mut device_server_present: Option<bool> = None;
    let mut device_server_version: Option<String> = None;
    let mut server_running: Option<bool> = None;
    let mut forward_established: Option<bool> = None;

    if let (Some(a), Some(s)) = (&adb, &serial) {
        // 并行层2：设备端文件 / 版本 / 监听实测 / forward 登记（互不依赖）
        let ls = a.shell(s, "ls /data/local/tmp 2>/dev/null", Duration::from_secs(10));
        let ver = a.shell(
            s,
            "su -c '/data/local/tmp/frida-server --version' 2>/dev/null || echo unknown",
            Duration::from_secs(15),
        );
        let ss = port_listening(a, s, port);
        let fwd = a.run(&["forward", "--list"], Duration::from_secs(10));
        let (ls, ver, ss, fwd) = tokio::join!(ls, ver, ss, fwd);
        if let Ok(o) = ls {
            device_server_present = Some(o.stdout.contains("frida-server"));
        }
        if let Ok(o) = ver {
            let v = o.stdout.trim().trim_end_matches("unknown").trim();
            device_server_version = (!v.is_empty()).then(|| v.to_string());
        }
        server_running = Some(ss);
        if let Ok(o) = fwd {
            forward_established = Some(o.stdout.contains(&format!("tcp:{port}")));
        }
    }

    // 矩阵
    let mut matrix = Vec::new();
    if let Ok(entries) = std::fs::read_dir(crate::paths::frida_server_matrix_dir()) {
        for e in entries.flatten() {
            if e.path().is_dir() {
                matrix.push(e.file_name().to_string_lossy().to_string());
            }
        }
    }
    matrix.sort();

    let overall = if client_version.is_some()
        && serial.is_some()
        && device_server_present == Some(true)
        && device_server_version == client_version
        && server_running == Some(true)
    {
        "pass"
    } else if client_error.is_some() {
        "fail"
    } else {
        "warn"
    };

    ServerStatusReport {
        client_version,
        client_error,
        device_serial: serial,
        device_server_present,
        device_server_version,
        server_running,
        forward_established,
        matrix,
        port,
        overall: overall.into(),
    }
}

async fn port_listening(adb: &AdbBackend, serial: &str, port: u16) -> bool {
    if let Ok(o) = adb
        .shell(
            serial,
            &format!("ss -tlnp 2>/dev/null | grep ':{port}' || echo NO"),
            Duration::from_secs(10),
        )
        .await
    {
        return o.stdout.contains(&format!(":{port}")) && !o.stdout.contains("NO");
    }
    false
}

/// 安装并启动 frida-server（幂等：先清残留再建立；每步留证据，S-01/S-02/S-03/S-05）
pub async fn server_install(
    cfg: &AppConfig,
    frida: &FridaChannelB,
) -> Result<Vec<StepReport>, String> {
    let mut steps: Vec<StepReport> = Vec::new();
    let port = cfg.frida_port;

    let adb = AdbBackend::detect(&cfg.adb_path, &cfg.doctor.adb_extra_paths).await?;
    let devices = adb.devices().await?;
    let Some(serial) = devices
        .iter()
        .find(|d| d.state == "device")
        .map(|d| d.serial.clone())
    else {
        return Err("无 device 状态设备：请先连接模拟器（设备连接面板）".into());
    };
    steps.push(step("设备就绪", "pass", vec![format!("serial = {serial}")]));

    // ① 版本匹配：以本机客户端版本为准，在矩阵中找对应 ABI 的 frida-server
    let hello = frida.call("hello", json!({})).await?;
    let client_version = hello
        .get("frida")
        .and_then(|s| s.as_str())
        .ok_or("sidecar 未返回 frida 版本")?
        .to_string();
    let abi_out = adb
        .shell(
            &serial,
            "getprop ro.product.cpu.abi",
            Duration::from_secs(10),
        )
        .await?;
    let abi = abi_out.stdout.trim().to_string();
    let dirname = abi_to_dirname(&abi);
    let local = find_server_binary(cfg, &client_version, dirname).ok_or_else(|| {
        format!(
            "frida-server {client_version}（android-{dirname}）在 bin 矩阵与工作区均缺失（S-01 三处一致）：体检页「下载 frida-server」可按需获取（C1）。设备 ABI={abi}"
        )
    })?;
    steps.push(step(
        "版本匹配",
        "pass",
        vec![
            format!(
                "客户端 {client_version} ↔ 矩阵 {} ({dirname})",
                client_version
            ),
            format!("设备 ABI = {abi}"),
        ],
    ));

    // ② 清残留（幂等，S-03；[f] 自匹配技巧收敛在 adb.pkill_residue，A4b）
    let kill = adb.pkill_residue(&serial, "frida-server").await;
    steps.push(step(
        "清理残留",
        "pass",
        vec![format!(
            "pkill 已执行（{}）",
            kill.map(|o| o.stdout.trim().to_string())
                .unwrap_or_else(|e| e)
        )],
    ));
    tokio::time::sleep(Duration::from_millis(500)).await;

    // ③ 推送
    let push = adb
        .push(
            &serial,
            &local.display().to_string(),
            "/data/local/tmp/frida-server",
            Duration::from_secs(180),
        )
        .await?;
    if push.timed_out {
        return Err(format!("push 超时：{}", push.stderr));
    }
    steps.push(step(
        "推送 frida-server",
        "pass",
        vec![
            format!("{} → /data/local/tmp/frida-server", local.display()),
            push.stdout.trim().to_string(),
        ],
    ));

    // ④ chmod + 属主（root 场景直接 755）
    adb.shell(
        &serial,
        "su -c 'chmod 755 /data/local/tmp/frida-server'",
        Duration::from_secs(10),
    )
    .await
    .map_err(|e| format!("chmod 失败：{e}"))?;
    steps.push(step(
        "chmod 755",
        "pass",
        vec!["/data/local/tmp/frida-server".into()],
    ));

    // ⑤ 启动（nohup + 后台，防 SIGHUP，S-02/S-03 常驻托管的第一层；进程守护在会话层持续校验）
    adb.shell(
        &serial,
        "su -c 'nohup /data/local/tmp/frida-server >/dev/null 2>&1 &'",
        Duration::from_secs(10),
    )
    .await
    .map_err(|e| format!("启动失败：{e}"))?;
    steps.push(step(
        "启动",
        "pass",
        vec!["su -c nohup /data/local/tmp/frida-server &".into()],
    ));

    // ⑥ 实测监听（S-05：假绿灯防护，以 ss -tlnp 为准）
    tokio::time::sleep(Duration::from_millis(1200)).await;
    if port_listening(&adb, &serial, port).await {
        steps.push(step(
            "实测监听",
            "pass",
            vec![format!("设备端 :{port} 正在监听")],
        ));
    } else {
        steps.push(step(
            "实测监听",
            "fail",
            vec![
                format!("启动命令已执行但 :{port} 未监听（S-05）"),
                "常见原因：SELinux Enforcing 拦截、ABI 不匹配、旧进程未退出".into(),
            ],
        ));
        return Ok(steps);
    }

    crate::audit::audit(
        "frida_server_install",
        &serial,
        "done",
        "session-console",
        &format!("v{client_version}"),
    );
    Ok(steps)
}

/// 建立 adb forward 并做端到端 TCP 验证（S-06 强化：主机侧端口不可绑定时自动探测替换）。
#[derive(Debug, Clone, Serialize)]
pub struct ForwardInfo {
    pub step: StepReport,
    pub host_port: u16,
}

async fn bind_ok(port: u16) -> bool {
    tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .is_ok()
}

async fn find_bindable_host_port(start: u16) -> Option<u16> {
    // 依次扫 frida_port+1..+100、49152+、50000+（WinNAT 保留段漂移下 50000+ 通常可用）
    let ranges: Vec<(u16, u16)> = vec![
        (start.saturating_add(1), start.saturating_add(100)),
        (49152, 49252),
        (50000, 50200),
    ];
    for (a, b) in ranges {
        for p in a..=b {
            if bind_ok(p).await {
                return Some(p);
            }
        }
    }
    None
}

pub async fn forward_setup(
    cfg: &AppConfig,
    adb: &AdbBackend,
    serial: &str,
) -> Result<ForwardInfo, String> {
    let device_port = cfg.frida_port;
    let host_port = if bind_ok(device_port).await {
        device_port
    } else {
        find_bindable_host_port(device_port)
            .await
            .ok_or("主机侧无可绑定端口（WinNAT 保留段覆盖过宽）：请在设置中调整 frida_port")?
    };
    adb.run(
        &[
            "-s",
            serial,
            "forward",
            &format!("tcp:{host_port}"),
            &format!("tcp:{device_port}"),
        ],
        Duration::from_secs(10),
    )
    .await
    .map_err(|e| format!("forward 失败：{e}"))?;
    let list = adb
        .run(
            &["-s", serial, "forward", "--list"],
            Duration::from_secs(10),
        )
        .await?;
    let listed = list.stdout.contains(&format!("tcp:{host_port}"));
    // 端到端验证：本机 TCP 直连主机侧端口
    let reachable = tokio::net::TcpStream::connect(("127.0.0.1", host_port))
        .await
        .is_ok();
    let (status, evidence): (&str, Vec<String>) = if listed && reachable {
        (
            "pass",
            vec![format!(
                "adb forward tcp:{host_port}→tcp:{device_port} 已建立并实测可连通{}",
                if host_port != device_port {
                    "（主机侧端口自动替换，S-06）"
                } else {
                    ""
                }
            )],
        )
    } else if listed {
        (
            "warn",
            vec![format!(
                "forward 已登记但 127.0.0.1:{host_port} 连不通（frida-server 未运行？）"
            )],
        )
    } else {
        ("fail", vec!["forward --list 中未找到登记项".to_string()])
    };
    Ok(ForwardInfo {
        step: step("adb forward", status, evidence),
        host_port,
    })
}

/// 附加链路：设备就绪 → forward → attach → load core agent → 等待 hello（注入成功判据）
pub async fn attach(
    app: &tauri::AppHandle,
    state: &FridaState,
    cfg: &AppConfig,
    target: Value,
    case_name: Option<String>,
) -> Result<SessionSnapshot, String> {
    // 操作护栏：双击附加此前会跑两条完整链路（两个 agent 先后加载，仅最后一个 script_id 被 detach）
    let _op = state.op_lock.lock().await;
    {
        let mut s = state.session.lock().await;
        s.phase = SessionPhase::Discovering;
        push_ev(&mut s, "开始附加链路".to_string());
        s.updated_at = chrono::Local::now().format("%H:%M:%S%.3f").to_string();
        let _ = app.emit("session-state", s.clone());
    }

    let adb = attach_step(
        app,
        state,
        AdbBackend::detect(&cfg.adb_path, &cfg.doctor.adb_extra_paths).await,
        "adb 探测失败",
    )
    .await?;
    let devices = attach_step(app, state, adb.devices().await, "枚举设备失败").await?;
    let Some(serial) = devices
        .iter()
        .find(|d| d.state == "device")
        .map(|d| d.serial.clone())
    else {
        return fail(app, state, "无 device 状态设备").await;
    };
    {
        let mut s = state.session.lock().await;
        s.phase = SessionPhase::DeviceReady;
        s.device = Some(serial.clone());
        push_ev(&mut s, format!("设备就绪：{serial}"));
        let _ = app.emit("session-state", s.clone());
    }

    // frida-server 运行检查；未运行则自动安装链（幂等）
    if !port_listening(&adb, &serial, cfg.frida_port).await {
        let _ = app.emit("session-state", {
            let mut s = state.session.lock().await;
            push_ev(
                &mut s,
                format!("设备端 :{} 未监听 → 自动执行安装链", cfg.frida_port),
            );
            s.clone()
        });
        let steps = attach_step(
            app,
            state,
            server_install(cfg, &state.channel).await,
            "frida-server 安装链执行失败",
        )
        .await?;
        if steps.iter().any(|s| s.status == "fail") {
            return fail(app, state, "frida-server 自动安装失败（见安装报告）").await;
        }
    }
    {
        let mut s = state.session.lock().await;
        s.phase = SessionPhase::ServerUp;
        push_ev(&mut s, format!("frida-server 实测监听 :{}", cfg.frida_port));
        let _ = app.emit("session-state", s.clone());
    }

    let fwd = attach_step(
        app,
        state,
        forward_setup(cfg, &adb, &serial).await,
        "adb forward 建立失败",
    )
    .await?;
    if fwd.step.status == "fail" {
        return fail(app, state, "adb forward 建立失败").await;
    }
    {
        let mut s = state.session.lock().await;
        s.phase = SessionPhase::Forwarded;
        s.forward_host_port = Some(fwd.host_port);
        extend_ev(&mut s, fwd.step.evidence.clone().into_iter());
        let _ = app.emit("session-state", s.clone());
    }

    // ---- 通道选择（文档10 P3-4）：auto 先 B，B 不可用降级 C；当前通道必须 UI 明示 ----
    let use_c = match state.preferred_channel.as_str() {
        "a" => {
            return fail(
                app,
                state,
                "通道 A（Rust 原生绑定）为远期计划：当前可用通道为 B（Python sidecar）/ C（CLI 兜底），请在设置中调整",
            )
            .await
        }
        "c" => true,
        "b" => false,
        _ => match state.channel.subscribe().await {
            Ok(_) => false, // sidecar 可启动 → 通道 B
            Err(e) => {
                if crate::backends::frida_c::FridaChannelC::available().await {
                    let mut s = state.session.lock().await;
                    s.evidence
                        .push(format!("通道 B 不可用（{e}）→ 降级通道 C（CLI 兜底，仅观测）"));
                    let snap = s.clone();
                    drop(s);
                    let _ = app.emit("session-state", snap);
                    true
                } else {
                    return fail(
                        app,
                        state,
                        &format!("通道 B 不可用（{e}），且通道 C 依赖的 frida CLI 未找到：无可用通道"),
                    )
                    .await;
                }
            }
        },
    };

    let host_port = state
        .session
        .lock()
        .await
        .forward_host_port
        .unwrap_or(cfg.frida_port);

    if use_c {
        let rx = state.channel_c.subscribe();
        attach_step(
            app,
            state,
            state
                .channel_c
                .attach_and_load(
                    app.clone(),
                    "127.0.0.1",
                    host_port,
                    &target_display(&target),
                )
                .await,
            "通道C 附加失败",
        )
        .await?;
        {
            let mut s = state.session.lock().await;
            s.phase = SessionPhase::Attached;
            s.session_id = None;
            s.script_id = Some(crate::backends::frida_c::VIRTUAL_SCRIPT_ID);
            s.channel = "C".into();
            s.evidence
                .push("通道C（CLI 兜底）已附加：观测可用；探针/探索器/REPL 需通道B".into());
            let _ = app.emit("session-state", s.clone());
        }
        return wait_hello(
            app,
            state,
            rx,
            crate::backends::frida_c::VIRTUAL_SCRIPT_ID,
            &target,
            case_name,
            "c",
            "当前通道：C（CLI 兜底）",
        )
        .await;
    }

    // ---- 通道 B ----
    // 事件订阅必须在 attach 之前建立（broadcast 不回放历史）
    let rx = attach_step(
        app,
        state,
        state.channel.subscribe().await,
        "事件通道订阅失败",
    )
    .await?;
    let (session_id, script_id) = attach_step(
        app,
        state,
        attach_and_load_core(&state.channel, "127.0.0.1", host_port, target.clone()).await,
        "attach / 加载 core agent 失败",
    )
    .await?;

    {
        let mut s = state.session.lock().await;
        s.phase = SessionPhase::Attached;
        s.session_id = Some(session_id);
        s.script_id = Some(script_id);
        s.target = Some(target_display(&target));
        push_ev(
            &mut s,
            format!("attach 成功 session#{session_id}，core agent 已加载 script#{script_id}"),
        );
        let _ = app.emit("session-state", s.clone());
    }

    wait_hello(
        app,
        state,
        rx,
        script_id,
        &target,
        case_name,
        "b",
        "当前通道：B（Python sidecar）",
    )
    .await
}

/// 等待 hello（注入成功判据，6s 超时）→ 成功即 Running + 落库 + 开 trace。
/// B/C 两路共用：hello 循环与通道无关，事件形状一致（C 走 send-shim 回流）。
async fn wait_hello(
    app: &tauri::AppHandle,
    state: &FridaState,
    mut rx: tokio::sync::broadcast::Receiver<FridaEvent>,
    script_id: u64,
    target: &Value,
    case_name: Option<String>,
    channel_db: &str,
    channel_evidence: &str,
) -> Result<SessionSnapshot, String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(6);
    loop {
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return fail(
                app,
                state,
                "core agent 加载后 6s 内未收到 hello（注入成功判据失败）",
            )
            .await;
        }
        match tokio::time::timeout(deadline - now, rx.recv()).await {
            Err(_elapsed) => {
                return fail(app, state, "等待 hello 超时").await;
            }
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Err(_)) => return fail(app, state, "事件通道关闭").await,
            Ok(Ok(ev)) => {
                if let FridaEvent::Message {
                    script_id: sid,
                    kind,
                    payload,
                    ..
                } = &ev
                {
                    if *sid == script_id && kind == "send" {
                        if let Some(t) = payload
                            .as_ref()
                            .and_then(|p| p.get("t"))
                            .and_then(|t| t.as_str())
                        {
                            if t == "hello" {
                                // agent 协议版本对账（批次⑪④）：core.js 内嵌于 Rust 二进制、
                                // sidecar exe 独立分发，升级节奏不同——错配此前只会以字段缺失
                                // 静默劣化。软检查不阻断，但必须在日志与会话事件流里明示。
                                match payload
                                    .as_ref()
                                    .and_then(|p| p.get("proto"))
                                    .and_then(|v| v.as_u64())
                                {
                                    Some(v) if v == crate::backends::frida::AGENT_PROTO_VERSION => {
                                    }
                                    Some(other) => {
                                        tracing::warn!("[session] agent proto={other} 高于宿主支持的 {}——未知字段将被忽略", crate::backends::frida::AGENT_PROTO_VERSION);
                                        let mut s = state.session.lock().await;
                                        push_ev(&mut s, format!("⚠ agent 协议版本 proto={other} 高于宿主支持的 {}，建议同步升级", crate::backends::frida::AGENT_PROTO_VERSION));
                                    }
                                    None => {
                                        tracing::warn!(
                                            "[session] agent 未上报 proto 版本（旧版 core.js？）"
                                        );
                                        let mut s = state.session.lock().await;
                                        push_ev(&mut s, "⚠ agent 未上报协议版本（旧版 core.js？），新事件类型可能无法落证据".to_string());
                                    }
                                }
                                // 三段式持锁：spawn_blocking 落库与 trace.start（内部也有
                                // spawn_blocking）都不得持 session 锁跨 await——否则
                                // frida_session_status/frida_rpc/detach 全部阻塞在锁上
                                let (db_case, serial_db, tgt_display) = {
                                    let mut s = state.session.lock().await;
                                    s.phase = SessionPhase::Running;
                                    s.hello = payload.clone();
                                    push_ev(
                                        &mut s,
                                        format!(
                                            "hello 握手成功：frida {} / pid {} / java {:?}",
                                            payload
                                                .as_ref()
                                                .and_then(|p| p.get("frida"))
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("?"),
                                            payload
                                                .as_ref()
                                                .and_then(|p| p.get("pid"))
                                                .and_then(|v| v.as_u64())
                                                .unwrap_or(0),
                                            payload
                                                .as_ref()
                                                .and_then(|p| p.get("java"))
                                                .and_then(|v| v.as_str()),
                                        ),
                                    );
                                    push_ev(&mut s, channel_evidence.to_string());
                                    s.updated_at =
                                        chrono::Local::now().format("%H:%M:%S%.3f").to_string();
                                    let db_case =
                                        case_name.clone().unwrap_or_else(|| "默认案件".into());
                                    (db_case, s.device.clone(), target_display(target))
                                };
                                // 落库（P2-3）：case→device→target→session；失败不阻断分析（锁外执行）
                                let ch = channel_db.to_string();
                                let db_result = tauri::async_runtime::spawn_blocking(move || {
                                    crate::store::session_start(
                                        &db_case,
                                        serial_db.as_deref(),
                                        &tgt_display,
                                        &ch,
                                        "attach 成功（hello 握手通过）",
                                    )
                                })
                                .await;
                                {
                                    let mut s = state.session.lock().await;
                                    match db_result {
                                        Ok(Ok(id)) => s.db_session_id = Some(id),
                                        Ok(Err(e)) => {
                                            push_ev(&mut s, format!("会话落库跳过：{e}"));
                                            tracing::warn!("[session] 会话落库失败（不阻断）：{e}");
                                        }
                                        Err(e) => {
                                            push_ev(&mut s, format!("会话落库跳过：{e}"));
                                            tracing::warn!(
                                                "[session] 会话落库任务失败（不阻断）：{e}"
                                            );
                                        }
                                    }
                                    push_ev(&mut s, "trace run 已开启".to_string());
                                    s.updated_at =
                                        chrono::Local::now().format("%H:%M:%S%.3f").to_string();
                                    let snap = s.clone();
                                    let _ = app.emit("session-state", snap.clone());
                                    drop(s);
                                    let trace: State<
                                        std::sync::Arc<crate::services::trace::TraceState>,
                                    > = app.state();
                                    trace.start(snap.db_session_id).await;
                                    crate::audit::audit(
                                        "session_attach",
                                        snap.target.as_deref().unwrap_or(""),
                                        "done",
                                        "session-console",
                                        &format!("channel={channel_db} script#{script_id}"),
                                    );
                                    return Ok(snap);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn target_display(target: &Value) -> String {
    if let Some(pid) = target.as_u64() {
        return format!("pid:{pid}");
    }
    target.as_str().unwrap_or("unknown").to_string()
}

async fn fail(
    app: &tauri::AppHandle,
    state: &FridaState,
    msg: &str,
) -> Result<SessionSnapshot, String> {
    let mut s = state.session.lock().await;
    s.phase = SessionPhase::Failed;
    push_ev(&mut s, format!("✖ {msg}"));
    s.updated_at = chrono::Local::now().format("%H:%M:%S%.3f").to_string();
    let snap = s.clone();
    let _ = app.emit("session-state", snap.clone());
    Err(msg.to_string())
}

/// attach 链路的步骤错误统一过 fail()：phase 必须落到 Failed，不允许 `?` 直穿把会话留在中间态
async fn attach_step<T>(
    app: &tauri::AppHandle,
    state: &FridaState,
    r: Result<T, String>,
    what: &str,
) -> Result<T, String> {
    match r {
        Ok(v) => Ok(v),
        Err(e) => match fail(app, state, &format!("{what}：{e}")).await {
            Err(msg) => Err(msg),
            Ok(_) => unreachable!("fail 必定返回 Err"),
        },
    }
}

pub async fn detach(app: &tauri::AppHandle, state: &FridaState) -> Result<SessionSnapshot, String> {
    // 操作护栏：与 attach 互斥（防止分离中途并发附加），也与 graceful_shutdown 互斥
    let _op = state.op_lock.lock().await;
    // 阶段1：短锁读出会话要素，随即还锁——sidecar RPC（各 30s 超时）、trace 落盘、
    // 落库收尾都是慢操作，此前全程持锁，sidecar 挂死时 detach 最长阻塞持锁 ~60s+
    let (channel_tag, script_id, session_id, db_session_id) = {
        let s = state.session.lock().await;
        (
            s.channel.clone(),
            s.script_id,
            s.session_id,
            s.db_session_id,
        )
    };
    // 分离是尽力而为的清理：单步失败不阻断后续步骤，但必须留痕而不是吞掉
    let mut errors: Vec<String> = Vec::new();
    if channel_tag == "C" {
        // 通道C：CLI 子进程就是会话本体，杀进程即卸载
        state.channel_c.detach().await;
    } else {
        if let Some(script_id) = script_id {
            if let Err(e) = state
                .channel
                .call("unload_script", json!({"script_id": script_id}))
                .await
            {
                errors.push(format!("卸载脚本 script#{script_id} 失败：{e}"));
            }
        }
        if let Some(session_id) = session_id {
            if let Err(e) = state
                .channel
                .call("detach", json!({"session_id": session_id}))
                .await
            {
                errors.push(format!("detach session#{session_id} 失败：{e}"));
            }
        }
    }
    let trace: State<std::sync::Arc<crate::services::trace::TraceState>> = app.state();
    let trace_rid = trace.stop().await;
    // 落库收尾（P2-3，锁外执行）
    if let Some(db_id) = db_session_id {
        if let Err(e) = tauri::async_runtime::spawn_blocking(move || {
            crate::store::session_finish(db_id, "stopped", "用户主动分离")
        })
        .await
        {
            tracing::warn!("[session] 会话落库收尾失败（不阻断）：{e}");
        }
    }
    if !errors.is_empty() {
        crate::audit::audit(
            "session_detach",
            "app",
            "warn",
            "session-console",
            &errors.join("；"),
        );
    }
    // 阶段2：重新持锁落终态并 emit
    let snap = {
        let mut s = state.session.lock().await;
        if channel_tag == "C" {
            push_ev(&mut s, "通道C CLI 进程已终止（脚本随之卸载）".to_string());
        }
        s.phase = SessionPhase::Stopped;
        s.session_id = None;
        s.script_id = None;
        s.hello = None;
        s.db_session_id = None;
        push_ev(&mut s, "已分离（脚本卸载 + 会话 detach）".to_string());
        if let Some(rid) = trace_rid {
            push_ev(&mut s, format!("trace run {rid} 已落盘"));
        }
        extend_ev(&mut s, errors.into_iter());
        s.updated_at = chrono::Local::now().format("%H:%M:%S%.3f").to_string();
        let snap = s.clone();
        let _ = app.emit("session-state", snap.clone());
        snap
    };
    crate::audit::audit(
        "session_detach",
        "app",
        "done",
        "session-console",
        "用户主动分离",
    );
    Ok(snap)
}

/// 附加后发一条 ping → 期望 pong（消息回路自检）
pub async fn ping(state: &FridaState) -> Result<Value, String> {
    let s = state.session.lock().await;
    let Some(script_id) = s.script_id else {
        return Err("无活动脚本会话".into());
    };
    drop(s);
    state
        .channel
        .call(
            "post",
            json!({"script_id": script_id, "message": {"type": "ping", "data": {"ts": "now"}}}),
        )
        .await?;
    // pong 走事件流，前端直接展示；这里确认 post 已受理
    Ok(json!({"posted": true}))
}

/// 供 lib.rs 启动时挂的全局事件转发（sidecar → 前端 + trace 管线）。
/// sidecar 重启时 ensure() 会整体替换 broadcast channel（backends/frida.rs），
/// 旧 channel 的 recv 返回 Closed——外层必须重新订阅新 channel，否则 trace 落盘
/// 与前端时间轴自此静默失效（wait_hello 有自己的订阅，恰好会掩盖此问题）。
pub async fn forward_events(
    handle: tauri::AppHandle,
    frida: FridaChannelB,
    trace: std::sync::Arc<crate::services::trace::TraceState>,
) {
    loop {
        // 只订阅已存活的 sidecar，不主动拉起进程（首启/重启由首次 API 调用触发）
        if let Ok(mut rx) = frida.subscribe_existing().await {
            loop {
                match rx.recv().await {
                    Ok(ev) => {
                        let mut agent_error_payload: Option<Value> = None;
                        if let FridaEvent::Message {
                            script_id,
                            kind,
                            payload,
                            data_b64,
                            description,
                            stack,
                            ..
                        } = &ev
                        {
                            // agent 脚本级异常（kind=error 时 payload=None，description/stack
                            // 承载详情）此前不落证据文件——崩溃只活在 UI 实时流里（批次⑪②）：
                            // 合成 agent_error 记录进 trace 管线，证据链补上这一段
                            if kind == "error" && payload.is_none() {
                                agent_error_payload = Some(serde_json::json!({
                                    "t": "agent_error",
                                    "script_id": script_id,
                                    "description": description,
                                    "stack": stack,
                                }));
                            }
                            let payload_ref = agent_error_payload
                                .as_ref()
                                .unwrap_or(payload.as_ref().unwrap_or(&Value::Null));
                            crate::services::trace::on_agent_message(
                                &handle,
                                &trace,
                                payload_ref,
                                *script_id,
                                data_b64,
                            );
                        }
                        // 大包不进 webview：data_b64 只属于 trace 管线（dex 落盘后有
                        // dex-dumped 事件），MB 级 base64 走 IPC 会把 webview 打卡
                        const MAX_EVENT_B64: usize = 1 << 20;
                        let out = match &ev {
                            FridaEvent::Message {
                                data_b64: Some(d), ..
                            } if d.len() > MAX_EVENT_B64 => {
                                let mut redacted = ev.clone();
                                if let FridaEvent::Message { data_b64, .. } = &mut redacted {
                                    *data_b64 =
                                        Some(format!("<{d} bytes 已由 trace 管线处理，此处省略>"));
                                }
                                redacted
                            }
                            _ => ev.clone(),
                        };
                        // 直接序列化事件本身（此前 to_value + emit 序列化了两次）
                        let _ = handle.emit("frida-event", &out);
                        if let FridaEvent::Detached { reason, .. } = &ev {
                            crate::audit::audit(
                                "frida_detached",
                                "session",
                                "warn",
                                "sidecar",
                                reason,
                            );
                        }
                        // 请求生命周期留痕（批次⑩）：宿主已放弃的调用最终完成/失败必须可审计——
                        // 这是「UI 与真实探针状态分叉」类事故的唯一直接证据
                        if let FridaEvent::OpAbandoned { method, .. } = &ev {
                            tracing::warn!("[通道B] 请求弃管（25s 结构化超时）：{method}");
                        }
                        if let FridaEvent::OpLate {
                            req_id, method, ok, ..
                        } = &ev
                        {
                            tracing::warn!(
                                "[通道B] 弃管请求迟到完成：req={req_id} method={method} ok={ok}"
                            );
                            crate::audit::audit(
                                "op_late",
                                method,
                                if *ok { "warn" } else { "info" },
                                "sidecar",
                                &format!("req={req_id}"),
                            );
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("[frida] 事件积压丢弃 {n} 条");
                        // 丢弃发生在 trace 落盘之前：jsonl 补 gap 记录，证据文件必须能解释行号空洞
                        crate::services::trace::on_gap(&handle, &trace, n);
                    }
                    Err(_) => break, // channel 关闭：sidecar 已重启换新 channel，外层重订阅
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// 进程/应用枚举（会话控制台列表）
#[derive(Debug, Clone, Serialize)]
pub struct ProcEntry {
    pub pid: u32,
    pub name: String,
    pub group: String, // system | user
    pub running: bool,
}

/// 系统/用户进程分类（枚举两路共用：通道B sidecar / 通道C frida-ps）
fn proc_is_system(pid: u32, name: &str) -> bool {
    pid < 1000
        || name.starts_with("android.")
        || name.starts_with("com.android.")
        || name.starts_with("com.google.")
        || name.starts_with("com.qualcomm")
        || name.starts_with("com.mediatek")
        || name.starts_with("system")
}

pub async fn enumerate_processes(
    cfg: &AppConfig,
    frida: &FridaChannelB,
) -> Result<Vec<ProcEntry>, String> {
    // 先确保 forward 存在并取主机侧端口（S-06：主机端口可能与设备端口不同）
    let adb = AdbBackend::detect(&cfg.adb_path, &cfg.doctor.adb_extra_paths).await?;
    let devices = adb.devices().await?;
    let serial = devices
        .iter()
        .find(|d| d.state == "device")
        .map(|d| d.serial.clone())
        .ok_or("无 device 状态设备：先连接模拟器")?;
    let host_port = forward_setup(cfg, &adb, &serial).await?.host_port;
    let conn = frida
        .call(
            "remote_connect",
            json!({"host": "127.0.0.1", "port": host_port}),
        )
        .await?;
    let device = conn
        .get("key")
        .and_then(|k| k.as_str())
        .ok_or("未返回 device key")?
        .to_string();
    let procs = frida
        .call("enumerate_processes", json!({"device": device}))
        .await?;
    let apps = frida
        .call("enumerate_applications", json!({"device": device}))
        .await
        .ok()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    let running_identifiers: Vec<String> = apps
        .iter()
        .filter(|a| a.get("pid").and_then(|p| p.as_u64()).unwrap_or(0) > 0)
        .filter_map(|a| {
            a.get("identifier")
                .and_then(|i| i.as_str())
                .map(String::from)
        })
        .collect();

    let mut out: Vec<ProcEntry> = Vec::new();
    if let Some(list) = procs.as_array() {
        for p in list {
            let pid = p.get("pid").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            let name = p
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let system = proc_is_system(pid, &name);
            out.push(ProcEntry {
                pid,
                group: if system {
                    "system".into()
                } else {
                    "user".into()
                },
                name: name.clone(),
                running: running_identifiers.iter().any(|i| i == &name),
            });
        }
    }
    out.sort_by(|a, b| b.pid.cmp(&a.pid));
    Ok(out)
}

/// 通道C 兜底枚举：frida-ps 解析 → 同款分组（running 徽标不可得，恒 false）
pub fn proc_entries_from_pairs(pairs: Vec<(u32, String)>) -> Vec<ProcEntry> {
    let mut out: Vec<ProcEntry> = pairs
        .into_iter()
        .map(|(pid, name)| {
            let system = proc_is_system(pid, &name);
            ProcEntry {
                pid,
                group: if system {
                    "system".into()
                } else {
                    "user".into()
                },
                name,
                running: false,
            }
        })
        .collect();
    out.sort_by(|a, b| b.pid.cmp(&a.pid));
    out
}
