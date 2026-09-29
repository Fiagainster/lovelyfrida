use crate::config::AppConfig;

#[tauri::command]
pub async fn get_config() -> Result<AppConfig, String> {
    Ok(crate::config::get())
}

/// 前端整包提交设置（含 ui 主题等）；Rust 侧校验 + 落盘 + 审计。
#[tauri::command]
pub async fn update_config(config: AppConfig) -> Result<AppConfig, String> {
    let updated = crate::config::update(|c| {
        *c = config;
    })?;
    crate::audit::audit(
        "update_config",
        "config.toml",
        "done",
        "settings-view",
        "前端提交整包配置",
    );
    Ok(updated)
}
