//! cases.db（文档06 数据模型与存储）：SQLite 元数据库；
//! trace 大文件一律落 `cases/traces/<run_id>.jsonl`，库内只存索引。
//! 不允许「删库重建」：schema_version 逐级迁移（M0 建库；v2 起为真实迁移链，见 migrate）。
use rusqlite::Connection;

const SCHEMA_VERSION: i64 = 2;

const DDL: &str = r#"
CREATE TABLE IF NOT EXISTS cases (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    created_at TEXT NOT NULL,
    notes TEXT DEFAULT '',
    read_only_roots TEXT DEFAULT '[]'
);
CREATE TABLE IF NOT EXISTS devices (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    case_id INTEGER NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
    serial TEXT NOT NULL,
    model TEXT DEFAULT '',
    abi TEXT DEFAULT '',
    state TEXT DEFAULT '',
    last_seen TEXT DEFAULT ''
);
CREATE TABLE IF NOT EXISTS app_profiles (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    case_id INTEGER NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
    package TEXT NOT NULL,
    uid INTEGER,
    apk_path TEXT DEFAULT '',
    data_dirs TEXT DEFAULT '[]',
    secret_files TEXT DEFAULT '[]',
    secret_transform TEXT DEFAULT '',
    entry_gesture TEXT DEFAULT '',
    entry_coords TEXT DEFAULT '[]',
    probe_targets TEXT DEFAULT '[]',
    notes TEXT DEFAULT ''
);
CREATE TABLE IF NOT EXISTS targets (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    case_id INTEGER NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
    device_id INTEGER REFERENCES devices(id) ON DELETE SET NULL,
    app_profile_id INTEGER REFERENCES app_profiles(id) ON DELETE SET NULL,
    data_version TEXT DEFAULT '',
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS sessions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    target_id INTEGER NOT NULL REFERENCES targets(id) ON DELETE CASCADE,
    channel TEXT NOT NULL DEFAULT 'b',
    state TEXT NOT NULL DEFAULT 'idle',
    started_at TEXT DEFAULT '',
    ended_at TEXT DEFAULT '',
    detail TEXT DEFAULT ''
);
CREATE TABLE IF NOT EXISTS probes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    clazz TEXT NOT NULL,
    method TEXT NOT NULL,
    overload_policy TEXT NOT NULL DEFAULT 'all',
    arg_formatters TEXT DEFAULT '[]',
    max_len INTEGER NOT NULL DEFAULT 128,
    condition TEXT DEFAULT '',
    action TEXT NOT NULL DEFAULT 'log',
    on_class_missing TEXT NOT NULL DEFAULT 'skip',
    state TEXT NOT NULL DEFAULT 'declared',
    hits INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    started_at TEXT NOT NULL,
    ended_at TEXT DEFAULT '',
    status TEXT NOT NULL DEFAULT 'running',
    trace_path TEXT DEFAULT '',
    line_count INTEGER NOT NULL DEFAULT 0,
    probe_hit_counts TEXT DEFAULT '{}',
    first_ts TEXT DEFAULT '',
    last_ts TEXT DEFAULT ''
);
CREATE TABLE IF NOT EXISTS steps (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    seq INTEGER NOT NULL,
    action TEXT NOT NULL,
    command TEXT DEFAULT '',
    params TEXT DEFAULT '{}',
    result TEXT DEFAULT '',
    duration_ms INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS experiments (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    case_id INTEGER NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'open',
    created_at TEXT NOT NULL,
    report_json TEXT DEFAULT ''
);
CREATE TABLE IF NOT EXISTS experiment_cases (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    experiment_id INTEGER NOT NULL REFERENCES experiments(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    input_kind TEXT NOT NULL,
    input_value TEXT DEFAULT '',
    origin TEXT NOT NULL DEFAULT 'manual'
);
CREATE TABLE IF NOT EXISTS diff_results (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    experiment_id INTEGER NOT NULL REFERENCES experiments(id) ON DELETE CASCADE,
    matrix TEXT DEFAULT '[]',
    variance TEXT DEFAULT '[]',
    hypotheses TEXT DEFAULT '[]',
    confirmed TEXT DEFAULT '[]'
);
CREATE TABLE IF NOT EXISTS crypto_schemes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    case_id INTEGER NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
    family TEXT DEFAULT '',
    concat_order TEXT DEFAULT '',
    salt_form TEXT DEFAULT '',
    per_round_input TEXT DEFAULT '',
    iteration INTEGER DEFAULT 1,
    output_encoding TEXT DEFAULT '',
    self_test_passed INTEGER NOT NULL DEFAULT 0,
    real_device_passed INTEGER NOT NULL DEFAULT 0,
    evidence_refs TEXT DEFAULT '[]'
);
CREATE TABLE IF NOT EXISTS brute_jobs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    case_id INTEGER NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
    space TEXT DEFAULT '{}',
    candidates_total INTEGER DEFAULT 0,
    est_speed REAL DEFAULT 0,
    est_eta TEXT DEFAULT '',
    engine TEXT DEFAULT 'builtin_c',
    self_test_passed INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'created',
    hit_value TEXT DEFAULT '',
    reversed_verified INTEGER NOT NULL DEFAULT 0,
    perf_note TEXT DEFAULT ''
);
CREATE TABLE IF NOT EXISTS findings (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    case_id INTEGER NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
    question_id TEXT DEFAULT '',
    question TEXT NOT NULL,
    answer TEXT DEFAULT '',
    confidence TEXT NOT NULL DEFAULT 'low',
    evidence TEXT DEFAULT '[]',
    source TEXT DEFAULT '',
    artifact_ids TEXT DEFAULT '[]',
    screenshot_slot TEXT DEFAULT ''
);
CREATE TABLE IF NOT EXISTS artifacts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    case_id INTEGER NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    path TEXT NOT NULL,
    sha256 TEXT DEFAULT '',
    created_by TEXT DEFAULT '',
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_runs_session ON runs(session_id);
CREATE INDEX IF NOT EXISTS idx_findings_case ON findings(case_id);
CREATE INDEX IF NOT EXISTS idx_probes_session ON probes(session_id);
"#;

/// 逐级迁移（禁止删库重建）。DDL 始终是最终版 schema；老库按版本差补丁。
/// SQLite 的 ALTER TABLE 只支持 ADD COLUMN，重列需 rename-recreate（关 FK 执行）。
fn migrate(conn: &Connection, from: i64) -> Result<(), String> {
    if from < 2 {
        // v1→v2：experiments 增加实验报告 JSON 列（受控实验结果入库）
        let has_col: bool = conn
            .prepare("PRAGMA table_info(experiments)")
            .and_then(|mut s| {
                let mut rows = s.query([])?;
                let mut found = false;
                while let Some(r) = rows.next()? {
                    let name: String = r.get(1)?;
                    if name == "report_json" {
                        found = true;
                    }
                }
                Ok(found)
            })
            .map_err(|e| e.to_string())?;
        if !has_col {
            conn.execute_batch("ALTER TABLE experiments ADD COLUMN report_json TEXT DEFAULT '';")
                .map_err(|e| format!("v1→v2 迁移失败：{e}"))?;
        }
    }
    Ok(())
}

pub fn init() -> Result<(), String> {
    let path = crate::paths::cases_db_path();
    let conn = Connection::open(&path).map_err(|e| format!("打开 cases.db 失败：{e}"))?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| e.to_string())?;
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(|e| e.to_string())?;
    conn.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|e| e.to_string())?;
    conn.execute_batch(DDL)
        .map_err(|e| format!("建表失败：{e}"))?;

    // schema_version 记录（逐级迁移的锚点；禁止删库重建）
    conn.execute(
        "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
        [],
    )
    .map_err(|e| e.to_string())?;
    let existing: Option<String> = conn
        .query_row(
            "SELECT value FROM meta WHERE key='schema_version'",
            [],
            |r| r.get(0),
        )
        .map(Some)
        .or_else(|e| {
            if e == rusqlite::Error::QueryReturnedNoRows {
                Ok(None)
            } else {
                Err(e)
            }
        })
        .map_err(|e| e.to_string())?;
    let current = existing
        .as_ref()
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(if existing.is_some() { i64::MAX } else { 0 });

    // 既有库版本高于代码 → 拒绝打开（防降级写入）
    if existing.is_some() && current > SCHEMA_VERSION {
        return Err(format!(
            "cases.db schema_version={current} 高于当前程序支持的 {SCHEMA_VERSION}，请升级程序"
        ));
    }
    // 老库逐级迁移；新库（无版本记录）直接落当前版本
    if existing.is_some() && current < SCHEMA_VERSION {
        migrate(&conn, current)?;
    }
    conn.execute(
        "INSERT INTO meta(key, value) VALUES ('schema_version', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [SCHEMA_VERSION.to_string()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ---------------- 落库辅助（P2-3，文档10：15 张表从 3 张有写入到核心链路全落库） ----------------
// 全部自带 Connection（调用方在 spawn_blocking 里跑）；失败由调用方决定降级（不阻断主流程）。
// 唯一连接构造入口（批次⑪①）：history/ledger/extras_svc 一律走这里——此前三处自建连接
// 缺 busy_timeout（rusqlite 默认 0ms），与 attach 落库并发时瞬时 SQLITE_BUSY 直接报给用户。
pub(crate) fn open_db() -> Result<Connection, String> {
    let conn = Connection::open(crate::paths::cases_db_path())
        .map_err(|e| format!("打开 cases.db 失败：{e}"))?;
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(|e| e.to_string())?;
    // 多写者并发（attach 落库撞 ledger/trace 写入）时等锁而不是立即 SQLITE_BUSY：
    // rusqlite 默认 0ms，而所有调用方按「落库失败不阻断」降级——没这行等于静默丢数据
    conn.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|e| e.to_string())?;
    Ok(conn)
}

/// 落库收尾统一入口（批次⑪①）：失败一律 tracing::warn 留痕，不阻断主流程。
/// 此前全链 `let _ =` 静默吞错——「落库失败不阻断」的设计本意是别拖垮分析会话，
/// 但取证数据的丢失必须可在日志里对账，否则与「可审计」承诺直接矛盾。
pub(crate) fn logged<T>(op: &str, f: impl FnOnce() -> Result<T, String>) -> Option<T> {
    match f() {
        Ok(v) => Some(v),
        Err(e) => {
            tracing::warn!("[落库] {op} 失败（数据未入库，主流程继续）：{e}");
            None
        }
    }
}

pub(crate) fn ensure_case_row(conn: &Connection, case_name: &str) -> Result<i64, String> {
    let mut stmt = conn
        .prepare("SELECT id FROM cases WHERE name = ?1")
        .map_err(|e| e.to_string())?;
    let existing: Option<i64> = stmt
        .query_row([case_name], |r| r.get(0))
        .map(Some)
        .or_else(|e| {
            if e == rusqlite::Error::QueryReturnedNoRows {
                Ok(None)
            } else {
                Err(e)
            }
        })
        .map_err(|e| e.to_string())?;
    if let Some(id) = existing {
        return Ok(id);
    }
    conn.execute(
        "INSERT INTO cases(name, created_at) VALUES (?1, ?2)",
        rusqlite::params![case_name, chrono::Local::now().to_rfc3339()],
    )
    .map_err(|e| e.to_string())?;
    Ok(conn.last_insert_rowid())
}

/// 会话开始：case→device→target→session 四级链一次性建齐，返回 session 行 id。
pub fn session_start(
    case_name: &str,
    device_serial: Option<&str>,
    target_display: &str,
    channel: &str,
    detail: &str,
) -> Result<i64, String> {
    let conn = open_db()?;
    let case_id = ensure_case_row(&conn, case_name)?;
    let device_id = match device_serial {
        Some(serial) => {
            let found: Option<i64> = conn
                .query_row(
                    "SELECT id FROM devices WHERE case_id = ?1 AND serial = ?2",
                    rusqlite::params![case_id, serial],
                    |r| r.get(0),
                )
                .map(Some)
                .or_else(|e| {
                    if e == rusqlite::Error::QueryReturnedNoRows {
                        Ok(None)
                    } else {
                        Err(e)
                    }
                })
                .map_err(|e| e.to_string())?;
            match found {
                Some(id) => {
                    let _ = conn.execute(
                        "UPDATE devices SET last_seen = ?2, state = 'device' WHERE id = ?1",
                        rusqlite::params![id, chrono::Local::now().to_rfc3339()],
                    );
                    Some(id)
                }
                None => {
                    conn.execute(
                        "INSERT INTO devices(case_id, serial, state, last_seen) VALUES (?1, ?2, 'device', ?3)",
                        rusqlite::params![case_id, serial, chrono::Local::now().to_rfc3339()],
                    )
                    .map_err(|e| e.to_string())?;
                    Some(conn.last_insert_rowid())
                }
            }
        }
        None => None,
    };
    conn.execute(
        "INSERT INTO targets(case_id, device_id, created_at) VALUES (?1, ?2, ?3)",
        rusqlite::params![case_id, device_id, chrono::Local::now().to_rfc3339()],
    )
    .map_err(|e| e.to_string())?;
    let target_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO sessions(target_id, channel, state, started_at, detail) VALUES (?1, ?2, 'running', ?3, ?4)",
        rusqlite::params![
            target_id,
            channel,
            chrono::Local::now().to_rfc3339(),
            format!("{detail}｜target={target_display}")
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(conn.last_insert_rowid())
}

pub fn session_finish(session_id: i64, state: &str, detail: &str) {
    logged("session_finish", || {
        let conn = open_db()?;
        conn.execute(
            "UPDATE sessions SET state = ?2, ended_at = ?3, detail = detail || ?4 WHERE id = ?1",
            rusqlite::params![
                session_id,
                state,
                chrono::Local::now().to_rfc3339(),
                format!("｜{detail}")
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    });
}

/// trace run 开启；session 未落库（db_session_id=None）时跳过，trace 文件照常写。
pub fn run_start(session_id: Option<i64>, trace_path: &str, started_at: &str) -> Option<i64> {
    let session_id = session_id?;
    logged("run_start", || {
        let conn = open_db()?;
        conn.execute(
            "INSERT INTO runs(session_id, started_at, status, trace_path) VALUES (?1, ?2, 'running', ?3)",
            rusqlite::params![session_id, started_at, trace_path],
        )
        .map_err(|e| e.to_string())?;
        Ok(conn.last_insert_rowid())
    })
}

pub fn run_finish(run_id: Option<i64>, line_count: u64, status: &str, ended_at: &str) {
    let Some(run_id) = run_id else { return };
    logged("run_finish", || {
        let conn = open_db()?;
        conn.execute(
            "UPDATE runs SET status = ?2, ended_at = ?3, line_count = ?4 WHERE id = ?1",
            rusqlite::params![run_id, status, ended_at, line_count as i64],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    });
}

/// 受控实验记录入库（v2：experiments.report_json）。
pub fn experiment_record(case_name: &str, title: &str, report_json: &str, cases_json: &str) {
    logged("experiment_record", || {
        let conn = open_db()?;
        let case_id = ensure_case_row(&conn, case_name)?;
        conn.execute(
            "INSERT INTO experiments(case_id, title, status, created_at, report_json) VALUES (?1, ?2, 'done', ?3, ?4)",
            rusqlite::params![case_id, title, chrono::Local::now().to_rfc3339(), report_json],
        )
        .map_err(|e| e.to_string())?;
        let experiment_id = conn.last_insert_rowid();
        if let Ok(arr) = serde_json::from_str::<serde_json::Value>(cases_json) {
            if let Some(list) = arr.as_array() {
                for c in list {
                    let name = c.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    let content = c.get("content").and_then(|v| v.as_str()).unwrap_or("");
                    if let Err(e) = conn.execute(
                        "INSERT INTO experiment_cases(experiment_id, name, input_kind, input_value, origin) VALUES (?1, ?2, 'template', ?3, 'manual')",
                        rusqlite::params![experiment_id, name, content],
                    ) {
                        tracing::warn!("[落库] experiment_cases 行写入失败（experiment_id={experiment_id}）：{e}");
                    }
                }
            }
        }
        Ok(())
    });
}

pub fn crypto_scheme_record(
    case_name: &str,
    family: &str,
    concat: &str,
    salt_form: &str,
    chain_input: &str,
    iterations: u64,
    output_encoding: &str,
    self_test_passed: bool,
    evidence_refs: &str,
) {
    logged("crypto_scheme_record", || {
        let conn = open_db()?;
        let case_id = ensure_case_row(&conn, case_name)?;
        conn.execute(
            "INSERT INTO crypto_schemes(case_id, family, concat_order, salt_form, per_round_input, iteration, output_encoding, self_test_passed, evidence_refs)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                case_id, family, concat, salt_form, chain_input, iterations, output_encoding,
                self_test_passed as i64, evidence_refs
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    });
}

#[allow(clippy::too_many_arguments)]
pub fn brute_job_record(
    case_name: &str,
    space_json: &str,
    candidates_total: u64,
    est_speed: f64,
    engine: &str,
    self_test_passed: bool,
    status: &str,
    hit_value: &str,
    reversed_verified: bool,
    perf_note: &str,
) {
    logged("brute_job_record", || {
        let conn = open_db()?;
        let case_id = ensure_case_row(&conn, case_name)?;
        conn.execute(
            "INSERT INTO brute_jobs(case_id, space, candidates_total, est_speed, engine, self_test_passed, status, hit_value, reversed_verified, perf_note)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            rusqlite::params![
                case_id, space_json, candidates_total as i64, est_speed, engine,
                self_test_passed as i64, status, hit_value, reversed_verified as i64, perf_note
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    });
}

pub fn artifact_record(case_name: &str, kind: &str, path: &str, sha256: &str, created_by: &str) {
    logged("artifact_record", || {
        let conn = open_db()?;
        let case_id = ensure_case_row(&conn, case_name)?;
        conn.execute(
            "INSERT INTO artifacts(case_id, kind, path, sha256, created_by, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![case_id, kind, path, sha256, created_by, chrono::Local::now().to_rfc3339()],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_migrate_v1_to_v2() {
        // 模拟 v1 库：老 experiments 表（无 report_json）→ 迁移后补列
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE experiments (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                case_id INTEGER NOT NULL,
                title TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'open',
                created_at TEXT NOT NULL
             );
             CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             INSERT INTO meta VALUES ('schema_version', '1');
             INSERT INTO experiments(case_id, title, created_at) VALUES (1, '老实验', '2026-01-01');",
        )
        .unwrap();
        migrate(&conn, 1).unwrap();
        let has_col: bool = conn
            .prepare("PRAGMA table_info(experiments)")
            .unwrap()
            .query_map([], |r| {
                let name: String = r.get(1)?;
                Ok(name == "report_json")
            })
            .unwrap()
            .any(|x| x.unwrap_or(false));
        assert!(has_col, "v1→v2 迁移必须补上 report_json 列");
        // 老数据仍在
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM experiments", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
    }
}
