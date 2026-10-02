//! 终端与 Recorder 命令
use crate::services::recorder::{clear, export, ExportResult, RecordedStep, RecorderState};
use crate::services::terminal::{self, TerminalInfo, TerminalMgr};
use tauri::State;

// ---------- 终端 ----------

#[tauri::command]
pub async fn terminal_create(
    app: tauri::AppHandle,
    mgr: State<'_, TerminalMgr>,
    serial: String,
) -> Result<TerminalInfo, String> {
    terminal::create(app, &mgr, &serial).await
}

#[tauri::command]
pub async fn terminal_write(
    mgr: State<'_, TerminalMgr>,
    recorder: State<'_, crate::services::recorder::RecorderState>,
    id: u32,
    data: String,
) -> Result<(), String> {
    terminal::write(&mgr, &recorder, id, &data).await
}

#[tauri::command]
pub async fn terminal_resize(
    mgr: State<'_, TerminalMgr>,
    id: u32,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    terminal::resize(&mgr, id, cols, rows).await
}

#[tauri::command]
pub async fn terminal_close(
    app: tauri::AppHandle,
    mgr: State<'_, TerminalMgr>,
    id: u32,
) -> Result<(), String> {
    terminal::close(app, &mgr, id).await
}

#[tauri::command]
pub async fn terminal_list(mgr: State<'_, TerminalMgr>) -> Result<Vec<TerminalInfo>, String> {
    Ok(terminal::list(&mgr).await)
}

// ---------- Recorder ----------

#[tauri::command]
pub async fn recorder_list(state: State<'_, RecorderState>) -> Result<Vec<RecordedStep>, String> {
    Ok(state.steps.lock().map_err(|_| "记录忙")?.clone())
}

#[tauri::command]
pub async fn recorder_export(
    state: State<'_, RecorderState>,
    format: String,
) -> Result<ExportResult, String> {
    export(&state, &format).await
}

#[tauri::command]
pub async fn recorder_clear(state: State<'_, RecorderState>) -> Result<(), String> {
    clear(state)
}
