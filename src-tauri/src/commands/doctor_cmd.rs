use crate::services::doctor::DoctorReport;
use crate::services::first_run::FirstRunReport;

/// 环境体检（10 项，文档04-A）。
/// quick（默认，<2s）：本地项 + adb 现状，不主动连接；deep（兜底）：端口 connect + offline 自愈。
#[tauri::command]
pub async fn doctor_run(deep: Option<bool>) -> Result<DoctorReport, String> {
    let cfg = crate::config::get();
    let report = crate::services::doctor::run(&cfg, deep.unwrap_or(false)).await;
    crate::audit::audit(
        "doctor_run",
        "environment",
        "done",
        "pipeline-view",
        &format!("mode={} overall={} {}ms", report.mode, report.overall, report.duration_ms),
    );
    Ok(report)
}

#[tauri::command]
pub async fn first_run_self_check() -> Result<FirstRunReport, String> {
    crate::services::first_run::run().await
}
