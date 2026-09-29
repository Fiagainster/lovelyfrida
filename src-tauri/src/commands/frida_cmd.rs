use crate::backends::adb::AdbBackend;
use crate::services::session::{
    attach, detach, enumerate_processes, forward_setup, ping, server_install, server_status,
    FridaState, ProcEntry, SessionSnapshot, ServerStatusReport, StepReport,
};

/// frida 环境总览（版本三处一致 + 运行状态 + forward + 矩阵）
#[tauri::command]
pub async fn frida_server_status(
    state: tauri::State<'_, FridaState>,
) -> Result<ServerStatusReport, String> {
    let cfg = crate::config::get();
    Ok(server_status(&cfg, &state.channel).await)
}

/// 安装并启动设备端 frida-server（幂等，返回逐步报告）
#[tauri::command]
pub async fn frida_server_install(
    state: tauri::State<'_, FridaState>,
) -> Result<Vec<StepReport>, String> {
    let cfg = crate::config::get();
    server_install(&cfg, &state.channel).await
}

/// 建立 adb forward 并端到端验证
#[tauri::command]
pub async fn frida_forward_setup() -> Result<StepReport, String> {
    let cfg = crate::config::get();
    let adb = AdbBackend::detect(&cfg.adb_path, &cfg.doctor.adb_extra_paths).await?;
    let devices = adb.devices().await?;
    let serial = devices
        .iter()
        .find(|d| d.state == "device")
        .map(|d| d.serial.clone())
        .ok_or("无 device 状态设备")?;
    let info = forward_setup(&cfg, &adb, &serial).await?;
    Ok(info.step)
}

/// 进程/应用枚举（分组：system/user）
#[tauri::command]
pub async fn frida_processes(
    state: tauri::State<'_, FridaState>,
) -> Result<Vec<ProcEntry>, String> {
    let cfg = crate::config::get();
    enumerate_processes(&cfg, &state.channel).await
}

/// 附加到目标（pid 或进程名），走完整链路并等待 hello
#[tauri::command]
pub async fn frida_session_attach(
    app: tauri::AppHandle,
    state: tauri::State<'_, FridaState>,
    target: serde_json::Value,
) -> Result<SessionSnapshot, String> {
    let cfg = crate::config::get();
    attach(&app, &state, &cfg, target).await
}

#[tauri::command]
pub async fn frida_session_detach(
    app: tauri::AppHandle,
    state: tauri::State<'_, FridaState>,
) -> Result<SessionSnapshot, String> {
    detach(&app, &state).await
}

#[tauri::command]
pub async fn frida_session_status(
    state: tauri::State<'_, FridaState>,
) -> Result<SessionSnapshot, String> {
    Ok(state.session.lock().await.clone())
}

/// 消息回路自检：post ping（pong 走事件流到前端）
#[tauri::command]
pub async fn frida_session_ping(state: tauri::State<'_, FridaState>) -> Result<serde_json::Value, String> {
    ping(&state).await
}
