//! cases.db（文档06 数据模型与存储）：SQLite 元数据库；
//! trace 大文件一律落 `cases/traces/<run_id>.jsonl`，库内只存索引。
//! 不允许「删库重建」：schema_version 逐级迁移（M0 建库，迁移在后续里程碑按需追加）。
use rusqlite::Connection;

const SCHEMA_VERSION: i64 = 1;

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
    created_at TEXT NOT NULL
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

pub fn init() -> Result<(), String> {
    let path = crate::paths::cases_db_path();
    let conn = Connection::open(&path).map_err(|e| format!("打开 cases.db 失败：{e}"))?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| e.to_string())?;
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(|e| e.to_string())?;
    conn.execute_batch(DDL)
        .map_err(|e| format!("建表失败：{e}"))?;

    // schema_version 记录（逐级迁移的锚点；禁止删库重建）
    conn.execute(
        "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
        [],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO meta(key, value) VALUES ('schema_version', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [SCHEMA_VERSION.to_string()],
    )
    .map_err(|e| e.to_string())?;

    // 读回校验（既有库版本高于代码版本时报警，防止降级写入）
    let v: i64 = conn
        .query_row("SELECT value FROM meta WHERE key='schema_version'", [], |r| {
            r.get::<_, String>(0)
                .and_then(|s| s.parse::<i64>().map_err(|_| rusqlite::Error::InvalidColumnType(0, "schema_version".into(), rusqlite::types::Type::Text)))
        })
        .unwrap_or(0);
    if v > SCHEMA_VERSION {
        return Err(format!(
            "cases.db schema_version={v} 高于当前程序支持的 {SCHEMA_VERSION}，请升级程序"
        ));
    }
    Ok(())
}
