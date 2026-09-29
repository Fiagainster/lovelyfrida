use crate::backends::adb::{AdbBackend, AdbCandidate, AdbDevice, ConnectReport};

#[derive(serde::Serialize)]
pub struct AdbResolveReport {
    pub path: Option<String>,
    pub source: String,
    pub candidates: Vec<AdbCandidate>,
}

#[tauri::command]
pub async fn resolve_adb() -> Result<AdbResolveReport, String> {
    let cfg = crate::config::get();
    match AdbBackend::detect(&cfg.adb_path, &cfg.doctor.adb_extra_paths).await {
        Ok(b) => Ok(AdbResolveReport {
            path: Some(b.path().display().to_string()),
            source: b.source,
            candidates: b.candidates,
        }),
        Err(e) => Err(e),
    }
}

#[tauri::command]
pub async fn adb_devices() -> Result<Vec<AdbDevice>, String> {
    let cfg = crate::config::get();
    let b = AdbBackend::detect(&cfg.adb_path, &cfg.doctor.adb_extra_paths).await?;
    b.devices().await
}

/// 单次连接：15s 硬超时（E-02）+ 二次确认（S-05）。
#[tauri::command]
pub async fn adb_connect(host: String, port: u16) -> Result<ConnectReport, String> {
    let cfg = crate::config::get();
    let b = AdbBackend::detect(&cfg.adb_path, &cfg.doctor.adb_extra_paths).await?;
    let report = b.connect(&host, port, cfg.adb_connect_timeout_s).await;
    crate::audit::audit(
        "adb_connect",
        &report.serial,
        if report.ok { "done" } else { "failed" },
        "device-connect-view",
        &format!("state={}", report.state),
    );
    Ok(report)
}

/// offline 自愈曲线（E-03）：disconnect → 2s → connect ×5。
#[tauri::command]
pub async fn adb_self_heal(host: String, port: u16) -> Result<ConnectReport, String> {
    let cfg = crate::config::get();
    let b = AdbBackend::detect(&cfg.adb_path, &cfg.doctor.adb_extra_paths).await?;
    let report = b.self_heal(&host, port, cfg.adb_connect_timeout_s).await;
    crate::audit::audit(
        "adb_self_heal",
        &report.serial,
        if report.ok { "done" } else { "failed" },
        "device-connect-view",
        &format!("attempts={}", report.attempts),
    );
    Ok(report)
}
