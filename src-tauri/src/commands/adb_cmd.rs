use crate::backends::adb::{AdbBackend, AdbCandidate, AdbDevice, ConnectReport};
use crate::services::recorder::RecorderState;

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
pub async fn adb_connect(
    recorder: tauri::State<'_, RecorderState>,
    host: String,
    port: u16,
) -> Result<ConnectReport, String> {
    let cfg = crate::config::get();
    let b = AdbBackend::detect(&cfg.adb_path, &cfg.doctor.adb_extra_paths).await?;
    let t0 = std::time::Instant::now();
    let report = b.connect(&host, port, cfg.adb_connect_timeout_s).await;
    let ms = t0.elapsed().as_millis() as u64;
    crate::audit::audit(
        "adb_connect",
        &report.serial,
        if report.ok { "done" } else { "failed" },
        "device-connect-view",
        &format!("state={}", report.state),
    );
    crate::services::recorder::record_cmd(
        &recorder,
        "adb 连接",
        &format!("adb connect {host}:{port}"),
        serde_json::json!({"host": host, "port": port}),
        if report.ok {
            format!("成功（{}）", report.state)
        } else {
            format!("失败（{}）", report.state)
        },
        ms,
    )
    .await;
    Ok(report)
}

/// offline 自愈曲线（E-03）：disconnect → 2s → connect ×5。
#[tauri::command]
pub async fn adb_self_heal(
    recorder: tauri::State<'_, RecorderState>,
    host: String,
    port: u16,
) -> Result<ConnectReport, String> {
    let cfg = crate::config::get();
    let b = AdbBackend::detect(&cfg.adb_path, &cfg.doctor.adb_extra_paths).await?;
    let t0 = std::time::Instant::now();
    let report = b.self_heal(&host, port, cfg.adb_connect_timeout_s).await;
    crate::services::recorder::record_cmd(
        &recorder,
        "adb 自愈重连",
        &format!("adb disconnect {host}:{port} && adb connect {host}:{port}（×5）"),
        serde_json::json!({"host": host, "port": port, "attempts": report.attempts}),
        if report.ok {
            "成功".into()
        } else {
            format!("失败（{} 轮）", report.attempts)
        },
        t0.elapsed().as_millis() as u64,
    )
    .await;
    crate::audit::audit(
        "adb_self_heal",
        &report.serial,
        if report.ok { "done" } else { "failed" },
        "device-connect-view",
        &format!("attempts={}", report.attempts),
    );
    Ok(report)
}
