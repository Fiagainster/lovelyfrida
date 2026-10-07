//! 审计日志（文档07）：时间(墙钟+单调) / 动作 / 目标 / 结果 / 触发来源 / 详情。
//! 覆盖：写操作、揭示明文、导出、执行命令。JSON Lines 追加写。
//!
//! 完整性（文档10 完善计划 P1-4）：
//! - 哈希链：每条 `hash = sha256(prev ‖ ‖核心字段 JSON)`，首条 prev 为空串，篡改任一条即断链
//! - 轮转：单文件超 10MB 改名 `audit-<ts>.log`，链头在内存中延续不断
//! - 脱敏：`mask_secrets_in_logs=true` 时 target/detail 过敏感键掩码（trace jsonl 证据文件不脱敏）
//! - 专用写线程：audit() 只入队，永不阻塞 async 运行时；写失败降级 tracing::warn 不静默
use chrono::Local;
use serde_json::json;
use std::sync::mpsc::Sender;
use std::sync::OnceLock;
use std::time::Instant;

static START: OnceLock<Instant> = OnceLock::new();

pub fn process_start() -> Instant {
    *START.get_or_init(Instant::now)
}

/// 敏感键（小写）。保守集合：不含 salt/key 这类本身常为公开的参数。
const SENSITIVE_KEYS: &[&str] = &[
    "password",
    "passwd",
    "pwd",
    "passphrase",
    "token",
    "secret",
    "credential",
    "apikey",
    "api_key",
    "auth",
];

fn eq_key_at(chars: &[char], pos: usize, key: &str) -> bool {
    key.chars().enumerate().all(|(n, kc)| {
        chars
            .get(pos + n)
            .is_some_and(|c| c.to_ascii_lowercase() == kc)
    })
}

/// 单 token 掩码：识别 `password=xxx` / `"password":"xxx"` 等形态。
/// 返回 (掩码结果, 值是否在下一个 token)——`token: abc` 这种键值被空格分开的形态
/// 交给 mask_secrets 用挂起机制处理。
fn mask_token(tok: &str) -> (String, bool) {
    let chars: Vec<char> = tok.chars().collect();
    let mut i = 0;
    while i < chars.len() && matches!(chars[i], '{' | '"' | '\'') {
        i += 1;
    }
    for &k in SENSITIVE_KEYS {
        if eq_key_at(&chars, i, k) {
            let mut j = i + k.len();
            while j < chars.len() && matches!(chars[j], '"' | '\'') {
                j += 1;
            }
            if j < chars.len() && (chars[j] == '=' || chars[j] == ':') {
                j += 1;
                while j < chars.len() && matches!(chars[j], '"' | '\'' | ' ') {
                    j += 1;
                }
                let mut end = chars.len();
                while end > j && matches!(chars[end - 1], '"' | '\'' | '}' | ',' | ')' | ';') {
                    end -= 1;
                }
                if end > j {
                    let mut out: String = chars[..j].iter().collect();
                    out.push_str("***");
                    out.extend(chars[end..].iter());
                    return (out, false);
                }
                // 键+分隔符齐了但值在下一个 token（如 `token: abc`）
                return (tok.to_string(), true);
            }
        }
    }
    (tok.to_string(), false)
}

/// 掩码独立值 token（挂起形态）：保留首尾装饰字符，内部替换为 ***
fn mask_value_token(tok: &str) -> String {
    let chars: Vec<char> = tok.chars().collect();
    let mut start = 0;
    while start < chars.len() && matches!(chars[start], '"' | '\'' | '{') {
        start += 1;
    }
    let mut end = chars.len();
    while end > start && matches!(chars[end - 1], '"' | '\'' | '}' | ',' | ')' | ';') {
        end -= 1;
    }
    if end > start {
        let mut out: String = chars[..start].iter().collect();
        out.push_str("***");
        out.extend(chars[end..].iter());
        out
    } else {
        "***".to_string()
    }
}

/// 按配置脱敏（日志/审计专用；trace jsonl 等证据文件不要用这里）
pub fn mask_secrets(s: &str) -> String {
    let cfg = crate::config::get();
    if !cfg.mask_secrets_in_logs {
        return s.to_string();
    }
    let mut out: Vec<String> = Vec::new();
    let mut value_pending = false;
    for tok in s.split_whitespace() {
        if value_pending {
            out.push(mask_value_token(tok));
            value_pending = false;
            continue;
        }
        let (masked, pending) = mask_token(tok);
        out.push(masked);
        value_pending = pending;
    }
    out.join(" ")
}

/// 哈希链：hash = sha256_hex(prev ‖ 0x1f ‖ 核心字段 JSON)。
/// 核心字段 JSON 由 serde_json 默认 BTreeMap 排序，验证方可稳定重算。
fn chain_hash(prev: &str, core: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(prev.as_bytes());
    h.update([0x1f]);
    h.update(core.as_bytes());
    format!("{:x}", h.finalize())
}

fn audit_log() -> std::path::PathBuf {
    crate::paths::audit_log_path()
}

/// 从现有日志尾行恢复链头（应用重启后链条延续而不是重置）
fn seed_chain() -> String {
    let Ok(content) = std::fs::read_to_string(audit_log()) else {
        return String::new();
    };
    for line in content.lines().rev() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            if let Some(h) = v.get("hash").and_then(|h| h.as_str()) {
                return h.to_string();
            }
        }
    }
    String::new()
}

fn writer(tx: std::sync::mpsc::Receiver<String>) {
    let mut prev = seed_chain();
    let max_size: u64 = 10 * 1024 * 1024;
    for core in tx {
        let hash = chain_hash(&prev, &core);
        let mut obj = serde_json::from_str::<serde_json::Value>(&core).unwrap_or(json!({}));
        if let Some(map) = obj.as_object_mut() {
            map.insert("prev".into(), json!(prev));
            map.insert("hash".into(), json!(hash));
        }
        let line = obj.to_string();

        let path = audit_log();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // 轮转：超限改名让位（链头在内存延续，跨轮转仍可验证）
        if let Ok(meta) = std::fs::metadata(&path) {
            if meta.len() > max_size {
                let ts = Local::now().format("%Y%m%d-%H%M%S");
                let rotated = path.with_file_name(format!("audit-{ts}.log"));
                let _ = std::fs::rename(&path, &rotated);
            }
        }
        match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            Ok(mut f) => {
                use std::io::Write;
                if let Err(e) = writeln!(f, "{line}") {
                    tracing::warn!("[audit] 写入失败（{e}）：{line}");
                }
            }
            Err(e) => tracing::warn!("[audit] 打开失败（{e}）：{line}"),
        }
        prev = hash;
    }
}

static SENDER: OnceLock<Sender<String>> = OnceLock::new();

/// 审计入口（非阻塞）：组装核心字段入队，由专用线程串行落盘 + 记链。
pub fn audit(action: &str, target: &str, result: &str, source: &str, detail: &str) {
    let entry = json!({
        "ts": Local::now().format("%Y-%m-%dT%H:%M:%S%.3f").to_string(),
        "ts_mono_ms": process_start().elapsed().as_millis() as u64,
        "action": action,
        "target": mask_secrets(target),
        "result": result,
        "source": source,
        "detail": mask_secrets(detail),
    });
    let tx = SENDER.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        let builder = std::thread::Builder::new().name("audit-writer".into());
        if let Err(e) = builder.spawn(move || writer(rx)) {
            tracing::error!("[audit] 写线程启动失败：{e}");
        }
        tx
    });
    if let Err(e) = tx.send(entry.to_string()) {
        tracing::warn!("[audit] 队列发送失败（写线程已退出）：{e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_covers_key_value_forms() {
        // 测试进程未 init config → get() 返回默认值，mask_secrets_in_logs=true
        assert_eq!(mask_secrets("password=Wei123123"), "password=***");
        assert_eq!(
            mask_secrets("{\"password\":\"Wei123123\"}"),
            "{\"password\":\"***\"}"
        );
        assert_eq!(mask_secrets("token: abc123;"), "token: ***;");
        assert_eq!(mask_secrets("plain text stays"), "plain text stays");
        assert_eq!(mask_secrets("salt=abc untouched"), "salt=abc untouched");
    }

    #[test]
    fn chain_hash_changes_with_prev() {
        assert_ne!(chain_hash("", "a"), chain_hash("x", "a"));
    }
}
