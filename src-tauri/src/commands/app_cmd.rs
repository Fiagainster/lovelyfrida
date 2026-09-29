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
pub async fn confirm_close(app: tauri::AppHandle) -> Result<(), String> {
    crate::graceful_shutdown(&app);
    Ok(())
}
