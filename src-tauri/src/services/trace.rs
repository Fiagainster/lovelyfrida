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
    file: Mutex<std::fs::File>,
    /// cases.db runs 行 id（P2-3；None=未落库，trace 文件照常写）
    pub db_run_id: Option<i64>,
    /// 已写入行数（stop 时回写 runs.line_count）
    pub lines: AtomicU64,
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
    /// 开启 trace run：打开 jsonl 文件 + 落 runs 行（P2-3，失败不阻断）。
    /// 注意：std Mutex 不得跨 await 持有——先快查、后重建锁插入。
    /// 已有未关闭的旧 run（attach 未 detach 就换了目标）先落 "superseded" 收尾再开新 run，
    /// 否则新会话的事件全部追加进旧 run 的 jsonl，runs.line_count 记在旧行上。
    pub async fn start(&self, db_session_id: Option<i64>) -> String {
        // take 出旧 run 并立即还锁（MutexGuard 临时值不得跨 await）
        let stale = self.run.lock().unwrap_or_else(|p| p.into_inner()).take();
        if let Some(stale) = stale {
            let lines = stale.lines.load(Ordering::SeqCst);
            let db_run_id = stale.db_run_id;
            let ended = chrono::Local::now().to_rfc3339();
            let _ = tauri::async_runtime::spawn_blocking(move || {
                crate::store::run_finish(db_run_id, lines, "superseded", &ended)
            })
            .await;
            tracing::warn!("[trace] 旧 run {} 未关闭即开新 run：旧 run 已按 superseded 收尾", stale.id);
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
            .open(&path);
        // runs 行入库：run_id 未拿到（库不可用/未落 session）也照常写文件
        let id_for_db = id.clone();
        let db_run_id = tauri::async_runtime::spawn_blocking(move || {
            crate::store::run_start(
                db_session_id,
                &format!("traces/run-{id_for_db}.jsonl"),
                &chrono::Local::now().to_rfc3339(),
            )
        })
        .await
        .ok()
        .flatten();
        let mut run = self.run.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(existing) = run.as_ref() {
            // 极端并发下以先到者为准（多插的一行 runs 无害，仅多一条孤儿索引）
            return existing.id.clone();
        }
        if let Ok(f) = file {
            *run = Some(RunState {
                id: id.clone(),
                file: Mutex::new(f),
                db_run_id,
                lines: AtomicU64::new(0),
            });
        }
        id
    }

    /// 结束 run：回写 runs 行（状态/结束时间/行数），返回 run id。
    pub async fn stop(&self) -> Option<String> {
        let run = self.run.lock().unwrap_or_else(|p| p.into_inner()).take()?;
        let lines = run.lines.load(Ordering::SeqCst);
        let db_run_id = run.db_run_id;
        let ended = chrono::Local::now().to_rfc3339();
        let _ = tauri::async_runtime::spawn_blocking(move || {
            crate::store::run_finish(db_run_id, lines, "done", &ended)
        })
        .await;
        Some(run.id)
    }

}

/// 单条记录：构建 + jsonl 落盘（一行一次 write，保留「行式追加，崩溃安全」语义）。
/// 只接受六类结构化观测事件；返回 None = 不属于 trace 范围。
fn write_record(run: &RunState, payload: &Value) -> Option<TraceRecord> {
    let t = payload.get("t").and_then(|v| v.as_str()).unwrap_or("");
    if !matches!(
        t,
        "probe_hit" | "probe_error" | "console" | "dlopen" | "register_natives" | "ssl_data"
    ) {
        return None;
    }
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
            if writeln!(f, "{line}").is_ok() {
                run.lines.fetch_add(1, Ordering::SeqCst);
            }
        }
    }
    Some(rec)
}

/// 事件入口：probe_hit / probe_error / console / dlopen / register_natives / ssl_data
/// 写入 trace；dex_dump 走内存 dex 落盘；batch（agent 批量层）逐条落盘后单次批量 emit。
pub fn on_agent_message(
    app: &tauri::AppHandle,
    trace: &TraceState,
    payload: &Value,
    script_id: u64,
    data_b64: &Option<String>,
) {
    let t = payload.get("t").and_then(|v| v.as_str()).unwrap_or("");
    // dex_dump：内存 dex 落盘（脱壳辅助）。base64 解码 + MB 级写盘是重 IO，放阻塞线程池
    if t == "dex_dump" && data_b64.is_some() {
        use base64::Engine;
        let app = app.clone();
        let data = data_b64.as_ref().unwrap().clone();
        let base = payload
            .get("base")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        tauri::async_runtime::spawn_blocking(move || {
            let dir = {
                let c = crate::config::get();
                crate::paths::cases_root(&c).join("dumps")
            };
            let _ = std::fs::create_dir_all(&dir);
            let fname = format!("dex-{}.dex", base.replace("0x", ""));
            let path = dir.join(&fname);
            if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&data) {
                if std::fs::write(&path, &bytes).is_ok() {
                    tracing::info!("[dex] 落盘 {} ({} bytes)", path.display(), bytes.len());
                    let _ = app.emit("dex-dumped", serde_json::json!({"path": path.display().to_string(), "size": bytes.len(), "base": base}));
                    crate::audit::audit("dex_dump", &path.display().to_string(), "done", "probe-lab", &format!("{} bytes", bytes.len()));
                }
            }
        });
        return;
    }
    // agent 批量层（O-02）：{t:"batch", items:[...]} —— 证据粒度不变（逐条落盘），
    // 但 IPC/序列化次数从「每事件一次」降到「每批次一次」
    if t == "batch" {
        if let Some(items) = payload.get("items").and_then(|v| v.as_array()) {
            // 批量层设计上不含 dex_dump（data 参数通道不可批量）；保险起见走单条入口
            for item in items {
                if item.get("t").and_then(|v| v.as_str()) == Some("dex_dump") {
                    on_agent_message(app, trace, item, script_id, &None);
                }
            }
            let mut records: Vec<TraceRecord> = Vec::new();
            let guard = trace.run.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(run) = guard.as_ref() {
                for item in items {
                    if item.get("t").and_then(|v| v.as_str()) == Some("dex_dump") {
                        continue;
                    }
                    if let Some(rec) = write_record(run, item) {
                        if item.get("t").and_then(|v| v.as_str()) == Some("probe_error") {
                            let id = item.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                            let err = item.get("error").and_then(|v| v.as_str()).unwrap_or("");
                            crate::audit::audit("probe_error", id, "warn", "probe-lab", err);
                        }
                        records.push(rec);
                    }
                }
            }
            drop(guard);
            if !records.is_empty() {
                let _ = app.emit(
                    "trace-event",
                    serde_json::json!({"t": "trace_batch", "records": records}),
                );
            }
        }
        return;
    }
    let guard = trace.run.lock().unwrap_or_else(|p| p.into_inner());
    let Some(run) = guard.as_ref() else { return };
    let Some(rec) = write_record(run, payload) else { return };
    drop(guard);
    let _ = app.emit("trace-event", rec);
    if t == "probe_error" {
        let id = payload.get("id").and_then(|v| v.as_str()).unwrap_or("?");
        let err = payload.get("error").and_then(|v| v.as_str()).unwrap_or("");
        crate::audit::audit("probe_error", id, "warn", "probe-lab", err);
    }
    let _ = script_id;
}

/// Lagged 丢弃留痕：broadcast 积压丢弃发生在消费端（trace 落盘之前），被丢的
/// 事件永远到不了 jsonl。补一条 gap 记录，让证据文件能解释行号空洞（取证可审计），
/// 同时推前端时间轴显式提示。
pub fn on_gap(app: &tauri::AppHandle, trace: &TraceState, lost: u64) {
    let guard = trace.run.lock().unwrap_or_else(|p| p.into_inner());
    let Some(run) = guard.as_ref() else { return };
    let seq = SEQ.fetch_add(1, Ordering::SeqCst);
    let rec = TraceRecord {
        seq,
        wall: chrono::Local::now().format("%H:%M:%S%.3f").to_string(),
        run_id: run.id.clone(),
        payload: serde_json::json!({"t": "trace_gap", "lost": lost, "note": "broadcast 积压丢弃，事件未能落盘"}),
    };
    if let Ok(mut f) = run.file.lock() {
        use std::io::Write;
        if let Ok(line) = serde_json::to_string(&rec) {
            if writeln!(f, "{line}").is_ok() {
                run.lines.fetch_add(1, Ordering::SeqCst);
            }
        }
    }
    drop(guard);
    let _ = app.emit("trace-event", rec);
}
