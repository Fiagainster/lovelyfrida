//! 会话管理（文档04-B）：托管「设备→frida-server→forward→attach→agent 注入」链路。
//! 状态机（文档02§五）：不回退——失败保留现场，可只重试该步。
use crate::backends::adb::AdbBackend;
use crate::backends::frida::{attach_and_load_core, FridaChannelB, FridaEvent};
use crate::config::AppConfig;
use serde::Serialize;
use serde_json::{json, Value};
use tauri::{Emitter, Manager, State};
use std::time::Duration;
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
        }
    }
}

/// tauri managed state
pub struct FridaState {
    pub channel: FridaChannelB,
    pub session: Mutex<SessionSnapshot>,
}

impl FridaState {
    pub fn new(python: String) -> Self {
        Self {
            channel: FridaChannelB::new(python),
            session: Mutex::new(SessionSnapshot::default()),
        }
    }
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
    StepReport { name: name.into(), status: status.into(), evidence }
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

    // ① 本机客户端（sidecar hello）
    let (client_version, client_error) = match frida.call("hello", json!({})).await {
        Ok(v) => (v.get("frida").and_then(|s| s.as_str()).map(String::from), None),
        Err(e) => (None, Some(e)),
    };

    // adb 现状
    let adb = AdbBackend::detect(&cfg.adb_path, &cfg.doctor.adb_extra_paths)
        .await
        .ok();
    let devices = match &adb {
        Some(a) => a.devices().await.unwrap_or_default(),
        None => Vec::new(),
    };
    let serial = devices.iter().find(|d| d.state == "device").map(|d| d.serial.clone());

    let mut device_server_present: Option<bool> = None;
    let mut device_server_version: Option<String> = None;
    let mut server_running: Option<bool> = None;
    let mut forward_established: Option<bool> = None;

    if let (Some(a), Some(s)) = (&adb, &serial) {
        // ② 设备端文件与版本
        if let Ok(o) = a
            .shell(s, "ls /data/local/tmp 2>/dev/null", Duration::from_secs(10))
            .await
        {
            device_server_present = Some(o.stdout.contains("frida-server"));
        }
        if let Ok(o) = a
            .shell(
                s,
                "su -c '/data/local/tmp/frida-server --version' 2>/dev/null || echo unknown",
                Duration::from_secs(15),
            )
            .await
        {
            let v = o.stdout.trim().trim_end_matches("unknown").trim();
            device_server_version = (!v.is_empty()).then(|| v.to_string());
        }
        // ③ 运行状态：ss -tlnp 实测监听（S-05 假绿灯防护）
        server_running = Some(port_listening(a, s, port).await);
        // ④ forward
        if let Ok(o) = a.run(&["forward", "--list"], Duration::from_secs(10)).await {
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
pub async fn server_install(cfg: &AppConfig, frida: &FridaChannelB) -> Result<Vec<StepReport>, String> {
    let mut steps: Vec<StepReport> = Vec::new();
    let port = cfg.frida_port;

    let adb = AdbBackend::detect(&cfg.adb_path, &cfg.doctor.adb_extra_paths).await?;
    let devices = adb.devices().await?;
    let Some(serial) = devices.iter().find(|d| d.state == "device").map(|d| d.serial.clone())
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
        .shell(&serial, "getprop ro.product.cpu.abi", Duration::from_secs(10))
        .await?;
    let abi = abi_out.stdout.trim().to_string();
    let dirname = abi_to_dirname(&abi);
    let local = crate::paths::frida_server_matrix_dir()
        .join(&client_version)
        .join(format!("android-{dirname}"))
        .join("frida-server");
    if !local.is_file() {
        return Err(format!(
            "bin\\frida-server\\{client_version}\\android-{dirname}\\frida-server 缺失（S-01 三处一致）：请下载放入后重试。设备 ABI={abi}"
        ));
    }
    steps.push(step("版本匹配", "pass", vec![
        format!("客户端 {client_version} ↔ 矩阵 {} ({dirname})", client_version),
        format!("设备 ABI = {abi}"),
    ]));

    // ② 清残留（幂等，S-03）
    // pkill 自匹配陷阱：[f] 技巧让执行 shell 的命令行不命中自身
    let kill = adb
        .shell(&serial, "su -c 'pkill -f [f]rida-server; echo done'", Duration::from_secs(10))
        .await;
    steps.push(step("清理残留", "pass", vec![format!(
        "pkill 已执行（{}）",
        kill.map(|o| o.stdout.trim().to_string()).unwrap_or_else(|e| e)
    )]));
    tokio::time::sleep(Duration::from_millis(500)).await;

    // ③ 推送
    let push = adb
        .push(&serial, &local.display().to_string(), "/data/local/tmp/frida-server", Duration::from_secs(180))
        .await?;
    if push.timed_out {
        return Err(format!("push 超时：{}", push.stderr));
    }
    steps.push(step("推送 frida-server", "pass", vec![
        format!("{} → /data/local/tmp/frida-server", local.display()),
        push.stdout.trim().to_string(),
    ]));

    // ④ chmod + 属主（root 场景直接 755）
    adb.shell(&serial, "su -c 'chmod 755 /data/local/tmp/frida-server'", Duration::from_secs(10))
        .await
        .map_err(|e| format!("chmod 失败：{e}"))?;
    steps.push(step("chmod 755", "pass", vec!["/data/local/tmp/frida-server".into()]));

    // ⑤ 启动（nohup + 后台，防 SIGHUP，S-02/S-03 常驻托管的第一层；进程守护在会话层持续校验）
    adb.shell(
        &serial,
        "su -c 'nohup /data/local/tmp/frida-server >/dev/null 2>&1 &'",
        Duration::from_secs(10),
    )
    .await
    .map_err(|e| format!("启动失败：{e}"))?;
    steps.push(step("启动", "pass", vec!["su -c nohup /data/local/tmp/frida-server &".into()]));

    // ⑥ 实测监听（S-05：假绿灯防护，以 ss -tlnp 为准）
    tokio::time::sleep(Duration::from_millis(1200)).await;
    if port_listening(&adb, &serial, port).await {
        steps.push(step("实测监听", "pass", vec![format!("设备端 :{port} 正在监听")]));
    } else {
        steps.push(step("实测监听", "fail", vec![
            format!("启动命令已执行但 :{port} 未监听（S-05）"),
            "常见原因：SELinux Enforcing 拦截、ABI 不匹配、旧进程未退出".into(),
        ]));
        return Ok(steps);
    }

    crate::audit::audit("frida_server_install", &serial, "done", "session-console", &format!("v{client_version}"));
    Ok(steps)
}

/// 建立 adb forward 并做端到端 TCP 验证（S-06 强化：主机侧端口不可绑定时自动探测替换）。
#[derive(Debug, Clone, Serialize)]
pub struct ForwardInfo {
    pub step: StepReport,
    pub host_port: u16,
}

async fn bind_ok(port: u16) -> bool {
    tokio::net::TcpListener::bind(("127.0.0.1", port)).await.is_ok()
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

pub async fn forward_setup(cfg: &AppConfig, adb: &AdbBackend, serial: &str) -> Result<ForwardInfo, String> {
    let device_port = cfg.frida_port;
    let host_port = if bind_ok(device_port).await {
        device_port
    } else {
        find_bindable_host_port(device_port)
            .await
            .ok_or("主机侧无可绑定端口（WinNAT 保留段覆盖过宽）：请在设置中调整 frida_port")?
    };
    adb.run(
        &["-s", serial, "forward", &format!("tcp:{host_port}"), &format!("tcp:{device_port}")],
        Duration::from_secs(10),
    )
    .await
    .map_err(|e| format!("forward 失败：{e}"))?;
    let list = adb.run(&["-s", serial, "forward", "--list"], Duration::from_secs(10)).await?;
    let listed = list.stdout.contains(&format!("tcp:{host_port}"));
    // 端到端验证：本机 TCP 直连主机侧端口
    let reachable = tokio::net::TcpStream::connect(("127.0.0.1", host_port)).await.is_ok();
    let (status, evidence): (&str, Vec<String>) = if listed && reachable {
        (
            "pass",
            vec![format!(
                "adb forward tcp:{host_port}→tcp:{device_port} 已建立并实测可连通{}",
                if host_port != device_port { "（主机侧端口自动替换，S-06）" } else { "" }
            )],
        )
    } else if listed {
        ("warn", vec![format!("forward 已登记但 127.0.0.1:{host_port} 连不通（frida-server 未运行？）")])
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
) -> Result<SessionSnapshot, String> {
    {
        let mut s = state.session.lock().await;
        s.phase = SessionPhase::Discovering;
        s.evidence.push("开始附加链路".into());
        s.updated_at = chrono::Local::now().format("%H:%M:%S%.3f").to_string();
        let _ = app.emit("session-state", s.clone());
    }

    let adb = AdbBackend::detect(&cfg.adb_path, &cfg.doctor.adb_extra_paths).await?;
    let devices = adb.devices().await?;
    let Some(serial) = devices.iter().find(|d| d.state == "device").map(|d| d.serial.clone())
    else {
        return fail(app, state, "无 device 状态设备").await;
    };
    {
        let mut s = state.session.lock().await;
        s.phase = SessionPhase::DeviceReady;
        s.device = Some(serial.clone());
        s.evidence.push(format!("设备就绪：{serial}"));
        let _ = app.emit("session-state", s.clone());
    }

    // frida-server 运行检查；未运行则自动安装链（幂等）
    if !port_listening(&adb, &serial, cfg.frida_port).await {
        let _ = app.emit("session-state", {
            let mut s = state.session.lock().await;
            s.evidence.push(format!("设备端 :{} 未监听 → 自动执行安装链", cfg.frida_port));
            s.clone()
        });
        let steps = server_install(cfg, &state.channel).await?;
        if steps.iter().any(|s| s.status == "fail") {
            return fail(app, state, "frida-server 自动安装失败（见安装报告）").await;
        }
    }
    {
        let mut s = state.session.lock().await;
        s.phase = SessionPhase::ServerUp;
        s.evidence.push(format!("frida-server 实测监听 :{}", cfg.frida_port));
        let _ = app.emit("session-state", s.clone());
    }

    let fwd = forward_setup(cfg, &adb, &serial).await?;
    if fwd.step.status == "fail" {
        return fail(app, state, "adb forward 建立失败").await;
    }
    {
        let mut s = state.session.lock().await;
        s.phase = SessionPhase::Forwarded;
        s.forward_host_port = Some(fwd.host_port);
        s.evidence.extend(fwd.step.evidence.clone());
        let _ = app.emit("session-state", s.clone());
    }

    // 事件订阅必须在 attach 之前建立（broadcast 不回放历史）
    let mut rx = state.channel.subscribe().await?;

    let host_port = state.session.lock().await.forward_host_port.unwrap_or(cfg.frida_port);
    let (session_id, script_id) =
        attach_and_load_core(&state.channel, "127.0.0.1", host_port, target.clone()).await?;

    {
        let mut s = state.session.lock().await;
        s.phase = SessionPhase::Attached;
        s.session_id = Some(session_id);
        s.script_id = Some(script_id);
        s.target = Some(target_display(&target));
        s.evidence.push(format!("attach 成功 session#{session_id}，core agent 已加载 script#{script_id}"));
        let _ = app.emit("session-state", s.clone());
    }

    // 等待 hello（注入成功判据，6s 超时）
    let deadline = tokio::time::Instant::now() + Duration::from_secs(6);
    loop {
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return fail(app, state, "core agent 加载后 6s 内未收到 hello（注入成功判据失败）").await;
        }
        match tokio::time::timeout(deadline - now, rx.recv()).await {
            Err(_elapsed) => {
                return fail(app, state, "等待 hello 超时").await;
            }
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Err(_)) => return fail(app, state, "事件通道关闭").await,
            Ok(Ok(ev)) => {
                if let FridaEvent::Message { script_id: sid, kind, payload, .. } = &ev {
                    if *sid == script_id && kind == "send" {
                        if let Some(t) = payload.as_ref().and_then(|p| p.get("t")).and_then(|t| t.as_str()) {
                            if t == "hello" {
                                let mut s = state.session.lock().await;
                                s.phase = SessionPhase::Running;
                                s.hello = payload.clone();
                                s.evidence.push(format!(
                                    "hello 握手成功：frida {} / pid {} / java {:?}",
                                    payload.as_ref().and_then(|p| p.get("frida")).and_then(|v| v.as_str()).unwrap_or("?"),
                                    payload.as_ref().and_then(|p| p.get("pid")).and_then(|v| v.as_u64()).unwrap_or(0),
                                    payload.as_ref().and_then(|p| p.get("java")).and_then(|v| v.as_str()),
                                ));
                                s.updated_at = chrono::Local::now().format("%H:%M:%S%.3f").to_string();
                                s.evidence.push("trace run 已开启".into());
                                let snap = s.clone();
                                let _ = app.emit("session-state", snap.clone());
                                let trace: State<std::sync::Arc<crate::services::trace::TraceState>> = app.state();
                                trace.start(&snap.target.clone().unwrap_or_default());
                                crate::audit::audit(
                                    "session_attach",
                                    snap.target.as_deref().unwrap_or(""),
                                    "done",
                                    "session-console",
                                    &format!("session#{session_id} script#{script_id}"),
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

fn target_display(target: &Value) -> String {
    if let Some(pid) = target.as_u64() {
        return format!("pid:{pid}");
    }
    target.as_str().unwrap_or("unknown").to_string()
}

async fn fail(app: &tauri::AppHandle, state: &FridaState, msg: &str) -> Result<SessionSnapshot, String> {
    let mut s = state.session.lock().await;
    s.phase = SessionPhase::Failed;
    s.evidence.push(format!("✖ {msg}"));
    s.updated_at = chrono::Local::now().format("%H:%M:%S%.3f").to_string();
    let snap = s.clone();
    let _ = app.emit("session-state", snap.clone());
    Err(msg.to_string())
}

pub async fn detach(app: &tauri::AppHandle, state: &FridaState) -> Result<SessionSnapshot, String> {
    let mut s = state.session.lock().await;
    let errors: Vec<String> = Vec::new();
    if let Some(script_id) = s.script_id {
        let _ = state.channel.call("unload_script", json!({"script_id": script_id})).await;
    }
    if let Some(session_id) = s.session_id {
        let _ = state.channel.call("detach", json!({"session_id": session_id})).await;
    }
    s.phase = SessionPhase::Stopped;
    s.session_id = None;
    s.script_id = None;
    s.hello = None;
    s.evidence.push("已分离（脚本卸载 + 会话 detach）".into());
    {
        let trace: State<std::sync::Arc<crate::services::trace::TraceState>> = app.state();
        if let Some(rid) = trace.stop() {
            s.evidence.push(format!("trace run {rid} 已落盘"));
        }
    }
    if !errors.is_empty() {
        s.evidence.extend(errors);
    }
    s.updated_at = chrono::Local::now().format("%H:%M:%S%.3f").to_string();
    let snap = s.clone();
    let _ = app.emit("session-state", snap.clone());
    crate::audit::audit("session_detach", "app", "done", "session-console", "用户主动分离");
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
        .call("post", json!({"script_id": script_id, "message": {"type": "ping", "data": {"ts": "now"}}}))
        .await?;
    // pong 走事件流，前端直接展示；这里确认 post 已受理
    Ok(json!({"posted": true}))
}

/// 供 lib.rs 启动时挂的全局事件转发（sidecar → 前端 + trace 管线）
pub async fn forward_events(
    handle: tauri::AppHandle,
    frida: FridaChannelB,
    trace: std::sync::Arc<crate::services::trace::TraceState>,
) {
    match frida.subscribe().await {
        Ok(mut rx) => {
            loop {
                match rx.recv().await {
                    Ok(ev) => {
                        let v = serde_json::to_value(&ev).unwrap_or(Value::Null);
                        if let FridaEvent::Message { script_id, payload, data_b64, .. } = &ev {
                            crate::services::trace::on_agent_message(&handle, &trace, payload.as_ref().unwrap_or(&Value::Null), *script_id, data_b64);
                        }
                        let _ = handle.emit("frida-event", v);
                        if let FridaEvent::Detached { reason, .. } = &ev {
                            crate::audit::audit("frida_detached", "session", "warn", "sidecar", reason);
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("[frida] 事件积压丢弃 {n} 条");
                    }
                    Err(_) => break,
                }
            }
        }
        Err(e) => tracing::warn!("[frida] 事件订阅失败：{e}（sidecar 将在首次调用时重启）"),
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

pub async fn enumerate_processes(cfg: &AppConfig, frida: &FridaChannelB) -> Result<Vec<ProcEntry>, String> {
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
        .call("remote_connect", json!({"host": "127.0.0.1", "port": host_port}))
        .await?;
    let device = conn.get("key").and_then(|k| k.as_str()).ok_or("未返回 device key")?.to_string();
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
        .filter_map(|a| a.get("identifier").and_then(|i| i.as_str()).map(String::from))
        .collect();

    let mut out: Vec<ProcEntry> = Vec::new();
    if let Some(list) = procs.as_array() {
        for p in list {
            let pid = p.get("pid").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let system = pid < 1000
                || name.starts_with("android.")
                || name.starts_with("com.android.")
                || name.starts_with("com.google.")
                || name.starts_with("com.qualcomm")
                || name.starts_with("com.mediatek")
                || name.starts_with("system");
            out.push(ProcEntry {
                pid,
                group: if system { "system".into() } else { "user".into() },
                name: name.clone(),
                running: running_identifiers.iter().any(|i| i == &name),
            });
        }
    }
    out.sort_by(|a, b| b.pid.cmp(&a.pid));
    Ok(out)
}
