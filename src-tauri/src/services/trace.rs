//! trace 管线（文档06）：probe_hit / probe_error / console 结构化事件
//! 落 `cases/traces/run-<id>.jsonl`（行式追加，崩溃安全）+ 推前端时间轴。
//! 大文件不进数据库（原则：SQLite 只存索引）。
use serde::Serialize;
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use tauri::Emitter;

static SEQ: AtomicU64 = AtomicU64::new(0);

pub struct RunState {
    pub id: String,
    #[allow(dead_code)] // 时间轴头部展示
    pub target: String,
    #[allow(dead_code)] // 时间轴头部展示
    pub started_at: String,
    file: Mutex<std::fs::File>,
}

#[derive(Default)]
pub struct TraceState {
    pub run: Mutex<Option<RunState>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TraceRecord {
    pub seq: u64,
    pub wall: String,
    pub run_id: String,
    pub payload: Value,
}

impl TraceState {
    pub fn start(&self, target: &str) -> String {
        let mut run = self.run.lock().unwrap_or_else(|p| p.into_inner());
        if run.is_some() {
            return run.as_ref().unwrap().id.clone();
        }
        let id = chrono::Local::now().format("%Y%m%d-%H%M%S%.3f").to_string();
        let dir = {
            let cfg = crate::config::get();
            crate::paths::cases_root(&cfg).join("traces")
        };
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!("run-{id}.jsonl"));
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path);
        if let Ok(f) = file {
            *run = Some(RunState {
                id: id.clone(),
                file: Mutex::new(f),
                target: target.into(),
                started_at: chrono::Local::now().format("%H:%M:%S%.3f").to_string(),
            });
        }
        id
    }

    pub fn stop(&self) -> Option<String> {
        let mut run = self.run.lock().unwrap_or_else(|p| p.into_inner());
        run.take().map(|r| r.id)
    }

    #[allow(dead_code)] // M2 后续 run 索引使用
    pub fn current_id(&self) -> Option<String> {
        self.run
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .map(|r| r.id.clone())
    }
}

/// 事件入口：命中/错误/console 三类写入 trace；其余忽略。
pub fn on_agent_message(app: &tauri::AppHandle, trace: &TraceState, payload: &Value, script_id: u64) {
    let t = payload.get("t").and_then(|v| v.as_str()).unwrap_or("");
    if !matches!(t, "probe_hit" | "probe_error" | "console") {
        return;
    }
    let guard = trace.run.lock().unwrap_or_else(|p| p.into_inner());
    let Some(run) = guard.as_ref() else { return };
    let seq = SEQ.fetch_add(1, Ordering::SeqCst);
    let rec = TraceRecord {
        seq,
        wall: chrono::Local::now().format("%H:%M:%S%.3f").to_string(),
        run_id: run.id.clone(),
        payload: payload.clone(),
    };
    if let Ok(mut f) = run.file.lock() {
        use std::io::Write;
        if let Ok(line) = serde_json::to_string(&rec) {
            let _ = writeln!(f, "{line}");
        }
    }
    drop(guard);
    let _ = app.emit("trace-event", rec);
    if t == "probe_error" {
        let id = payload.get("id").and_then(|v| v.as_str()).unwrap_or("?");
        let err = payload.get("error").and_then(|v| v.as_str()).unwrap_or("");
        crate::audit::audit("probe_error", id, "warn", "probe-lab", err);
    }
    let _ = script_id;
}
