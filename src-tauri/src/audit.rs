use chrono::Local;
use serde_json::json;
use std::io::Write;
use std::sync::OnceLock;
use std::time::Instant;

static START: OnceLock<Instant> = OnceLock::new();

pub fn process_start() -> Instant {
    *START.get_or_init(Instant::now)
}

/// 审计日志（文档07）：时间(墙钟+单调) / 动作 / 目标 / 结果 / 触发来源 / 详情。
/// 覆盖：写操作、揭示明文、导出、执行命令。JSON Lines 追加写。
pub fn audit(action: &str, target: &str, result: &str, source: &str, detail: &str) {
    let entry = json!({
        "ts": Local::now().format("%Y-%m-%dT%H:%M:%S%.3f").to_string(),
        "ts_mono_ms": process_start().elapsed().as_millis() as u64,
        "action": action,
        "target": target,
        "result": result,
        "source": source,
        "detail": detail,
    });
    let path = crate::paths::audit_log_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{entry}");
    }
}
