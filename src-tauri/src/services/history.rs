//! B2 落库数据面（文档10）：sessions/runs/experiments/brute_jobs 的只读查询。
//! P2-3 把数据写进了库，这里把它交还给 UI——否则库只是写入黑洞。
//! 全部同步 rusqlite，调用方（commands 层）负责 spawn_blocking。
use rusqlite::Connection;
use serde::Serialize;

// 连接构造收敛到 store::open_db（批次⑪①）：此前自建连接缺 busy_timeout/foreign_keys，
// 与写路径并发时瞬时 SQLITE_BUSY 直接报给用户
fn open() -> Result<Connection, String> {
    crate::store::open_db()
}

// ---------------- 会话历史 ----------------

#[derive(Debug, Clone, Serialize)]
pub struct SessionRow {
    pub id: i64,
    pub case_name: String,
    pub device_serial: Option<String>,
    /// 从 sessions.detail 的「target=…」段提取（P2-3 落库时的展示名）
    pub target: String,
    pub channel: String,
    pub state: String,
    pub started_at: String,
    pub ended_at: String,
}

pub fn list_sessions(limit: i64) -> Result<Vec<SessionRow>, String> {
    let conn = open()?;
    let mut stmt = conn
        .prepare(
            "SELECT s.id, c.name, d.serial, s.detail, s.channel, s.state, s.started_at, s.ended_at
             FROM sessions s
             JOIN targets t ON s.target_id = t.id
             JOIN cases c ON t.case_id = c.id
             LEFT JOIN devices d ON t.device_id = d.id
             ORDER BY s.id DESC LIMIT ?1",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([limit], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        let (id, case_name, device_serial, detail, channel, state, started_at, ended_at) =
            row.map_err(|e| e.to_string())?;
        // detail 形如「attach 成功…｜target=pid:123」：取 target= 之后的展示名
        let target = detail
            .split("target=")
            .nth(1)
            .map(|s| s.split('｜').next().unwrap_or(s).trim().to_string())
            .unwrap_or_default();
        out.push(SessionRow {
            id,
            case_name,
            device_serial,
            target,
            channel,
            state,
            started_at,
            ended_at,
        });
    }
    Ok(out)
}

// ---------------- trace run 历史 ----------------

#[derive(Debug, Clone, Serialize)]
pub struct RunRow {
    pub id: i64,
    pub session_id: i64,
    pub status: String,
    pub trace_path: String,
    pub line_count: i64,
    pub started_at: String,
    pub ended_at: String,
}

pub fn list_runs(limit: i64) -> Result<Vec<RunRow>, String> {
    let conn = open()?;
    let mut stmt = conn
        .prepare(
            "SELECT id, session_id, status, trace_path, line_count, started_at, ended_at
             FROM runs ORDER BY id DESC LIMIT ?1",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([limit], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        let (id, session_id, status, trace_path, line_count, started_at, ended_at) =
            row.map_err(|e| e.to_string())?;
        out.push(RunRow {
            id,
            session_id,
            status,
            trace_path,
            line_count,
            started_at,
            ended_at,
        });
    }
    Ok(out)
}

// ---------------- 实验历史 ----------------

#[derive(Debug, Clone, Serialize)]
pub struct ExperimentRow {
    pub id: i64,
    pub case_name: String,
    pub title: String,
    pub status: String,
    pub created_at: String,
    pub report_bytes: i64,
}

pub fn list_experiments(limit: i64) -> Result<Vec<ExperimentRow>, String> {
    let conn = open()?;
    let mut stmt = conn
        .prepare(
            "SELECT e.id, c.name, e.title, e.status, e.created_at, length(e.report_json)
             FROM experiments e JOIN cases c ON e.case_id = c.id
             ORDER BY e.id DESC LIMIT ?1",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([limit], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, Option<i64>>(5)?.unwrap_or(0),
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        let (id, case_name, title, status, created_at, report_bytes) =
            row.map_err(|e| e.to_string())?;
        out.push(ExperimentRow {
            id,
            case_name,
            title,
            status,
            created_at,
            report_bytes,
        });
    }
    Ok(out)
}

// ---------------- 爆破作业历史 ----------------

#[derive(Debug, Clone, Serialize)]
pub struct BruteJobRow {
    pub id: i64,
    pub case_name: String,
    pub engine: String,
    pub status: String,
    pub hit_value: String,
    pub candidates_total: i64,
    pub self_test_passed: bool,
    pub perf_note: String,
}

pub fn list_brute_jobs(limit: i64) -> Result<Vec<BruteJobRow>, String> {
    let conn = open()?;
    let mut stmt = conn
        .prepare(
            "SELECT b.id, c.name, b.engine, b.status, b.hit_value, b.candidates_total, b.self_test_passed, b.perf_note
             FROM brute_jobs b JOIN cases c ON b.case_id = c.id
             ORDER BY b.id DESC LIMIT ?1",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([limit], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, i64>(6)? != 0,
                r.get::<_, String>(7)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        let (
            id,
            case_name,
            engine,
            status,
            hit_value,
            candidates_total,
            self_test_passed,
            perf_note,
        ) = row.map_err(|e| e.to_string())?;
        out.push(BruteJobRow {
            id,
            case_name,
            engine,
            status,
            hit_value,
            candidates_total,
            self_test_passed,
            perf_note,
        });
    }
    Ok(out)
}
