//! Recorder v1（文档03§五 / 04-K）：可视化操作自动生成 Step（等价命令+参数+结果+耗时），
//! 支持导出 .ps1/.sh/.md/.json。终端手敲命令的捕获在 M1 后续版本接入。
use serde::Serialize;
use serde_json::Value;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use tauri::State;

static SEQ: AtomicU32 = AtomicU32::new(1);

pub async fn record_cmd(
    state: &RecorderState,
    action: &str,
    command: &str,
    params: Value,
    result: String,
    duration_ms: u64,
) {
    record(state, action, command, params, &result, duration_ms).await;
}

#[derive(Debug, Clone, Serialize)]
pub struct RecordedStep {
    pub seq: u32,
    pub ts: String,
    pub action: String,
    pub command: String,
    pub params: Value,
    pub result: String,
    pub duration_ms: u64,
}

#[derive(Default)]
pub struct RecorderState {
    pub steps: Mutex<Vec<RecordedStep>>,
}

pub async fn record(
    state: &RecorderState,
    action: &str,
    command: &str,
    params: Value,
    result: &str,
    duration_ms: u64,
) {
    let mut steps = match state.steps.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    steps.push(RecordedStep {
        seq: SEQ.fetch_add(1, Ordering::SeqCst),
        ts: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        action: action.into(),
        command: command.into(),
        params,
        result: result.into(),
        duration_ms,
    });
    if steps.len() > 500 {
        let over = steps.len() - 500;
        steps.drain(0..over);
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ExportResult {
    pub path: String,
    pub count: usize,
}

/// 导出（文档03：按当前 OS 出方言 + md 带截图位）。
/// 内容组装持锁内存完成，写盘走 spawn_blocking（P1-5）。
pub async fn export(state: &RecorderState, format: &str) -> Result<ExportResult, String> {
    let (path, content, count) = {
        let steps = state.steps.try_lock().map_err(|_| "记录忙")?;
        if steps.is_empty() {
            return Err("暂无操作记录".into());
        }
        let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let ext = match format {
            "ps1" | "sh" | "md" | "json" => format,
            other => return Err(format!("未知导出格式：{other}")),
        };
        let cfg = crate::config::get();
        let dir = crate::paths::cases_root(&cfg).join("exports");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let path = dir.join(format!("recording-{ts}.{ext}"));

        let mut content = String::new();
        match format {
            "json" => {
                content = serde_json::to_string_pretty(&*steps).map_err(|e| e.to_string())?;
            }
            "sh" => {
                content.push_str("#!/bin/sh\n# LovelyFrida 操作记录（可重放）\nset -e\n");
                for s in steps.iter() {
                    content.push_str(&format!("# step{} [{}] {}\n", s.seq, s.ts, s.action));
                    content.push_str(&s.command);
                    content.push('\n');
                }
            }
            "ps1" => {
                content.push_str("# LovelyFrida 操作记录（可重放）\r\n");
                for s in steps.iter() {
                    content.push_str(&format!("# step{} [{}] {}\r\n", s.seq, s.ts, s.action));
                    content.push_str(&s.command.replace('\n', " && "));
                    content.push_str("\r\n");
                }
            }
            "md" => {
                content.push_str("# LovelyFrida 操作记录\n\n> 由 Recorder v1 自动生成，可直接粘入笔记。\n\n");
                for (i, s) in steps.iter().enumerate() {
                    content.push_str(&format!(
                        "## {}. {}\n\n- 时间：`{}`\n- 耗时：{}ms\n- 结果：{}\n\n```sh\n{}\n```\n\n【截图位 {}-1】\n\n",
                        i + 1,
                        s.action,
                        s.ts,
                        s.duration_ms,
                        s.result,
                        s.command,
                        i + 1
                    ));
                }
            }
            other => return Err(format!("未知导出格式：{other}")),
        }
        (path, content, steps.len())
    };
    // 写路径过 guard（P1-1）：导出目标不得落在检材只读根
    crate::guard::guard_write_or_err(&path)?;
    let p = path.clone();
    tauri::async_runtime::spawn_blocking(move || std::fs::write(&p, &content))
        .await
        .map_err(|e| format!("后台任务失败：{e}"))?
        .map_err(|e| e.to_string())?;
    crate::audit::audit("recorder_export", &path.display().to_string(), "done", "recorder", format);
    Ok(ExportResult {
        path: path.display().to_string(),
        count,
    })
}

pub fn clear(state: State<'_, RecorderState>) -> Result<(), String> {
    state.steps.lock().map_err(|_| "记录忙")?.clear();
    Ok(())
}
