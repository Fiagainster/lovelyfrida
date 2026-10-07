use crate::state::AppState;
use serde::Serialize;

#[derive(Serialize)]
pub struct AppInfo {
    pub version: String,
    pub root: String,
    pub workspace_dir: String,
    pub cases_dir: String,
    pub logs_dir: String,
}

#[tauri::command]
pub async fn get_app_info() -> Result<AppInfo, String> {
    let cfg = crate::config::get();
    Ok(AppInfo {
        version: env!("CARGO_PKG_VERSION").into(),
        root: crate::paths::app_root().display().to_string(),
        workspace_dir: crate::paths::workspace_root(&cfg).display().to_string(),
        cases_dir: crate::paths::cases_root(&cfg).display().to_string(),
        logs_dir: crate::paths::logs_dir().display().to_string(),
    })
}

/// 关闭握手第二步：前端确认后执行优雅关停（M1 起先 detach/清理子进程）。
#[tauri::command]
pub async fn confirm_close(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    // 幂等门（A4b）：重复确认直接返回，避免二次关停审计/看门狗重臂
    if state.is_shutting() {
        return Ok(());
    }
    crate::graceful_shutdown(&app);
    Ok(())
}

/// 关闭握手取消：用户点了「取消」（或 ESC/蒙层关闭）——状态机拉回 IDLE，
/// 10s 看门狗不再保底强杀。仅 WAITING → IDLE；SHUTTING 不可逆（清理已在跑）。
#[tauri::command]
pub async fn cancel_close(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let _ = state.shutdown_phase.compare_exchange(
        crate::state::SHUTDOWN_WAITING,
        crate::state::SHUTDOWN_IDLE,
        std::sync::atomic::Ordering::SeqCst,
        std::sync::atomic::Ordering::SeqCst,
    );
    Ok(())
}
