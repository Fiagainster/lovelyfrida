//! 能力包 B/D（文档09 §脚本库 + U9）：用户脚本 CRUD + AppProfile CRUD + dump 索引。
//! 脚本运行 = agent replEval（钩子在 agent 内持久），运行记录进审计。
use rusqlite::Connection;
use serde::Serialize;

fn script_dir() -> std::path::PathBuf {
    let cfg = crate::config::get();
    crate::paths::cases_root(&cfg).join("scripts")
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 80
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || "._- ".contains(c))
}

// ---------------- 脚本库 ----------------

#[derive(Debug, Clone, Serialize)]
pub struct ScriptInfo {
    pub name: String,
    pub size: u64,
    pub modified: String,
}

pub fn script_list() -> Result<Vec<ScriptInfo>, String> {
    let dir = script_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for e in std::fs::read_dir(&dir).map_err(|e| e.to_string())? {
        let e = e.map_err(|e| e.to_string())?;
        let p = e.path();
        if p.extension().map(|x| x == "js").unwrap_or(false) {
            let meta = e.metadata().map_err(|e| e.to_string())?;
            let modified = meta
                .modified()
                .ok()
                .map(|t| {
                    chrono::DateTime::<chrono::Local>::from(t)
                        .format("%Y-%m-%d %H:%M")
                        .to_string()
                })
                .unwrap_or_default();
            out.push(ScriptInfo {
                name: p
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default(),
                size: meta.len(),
                modified,
            });
        }
    }
    out.sort_by(|a, b| b.modified.cmp(&a.modified));
    Ok(out)
}

pub fn script_read(name: &str) -> Result<String, String> {
    if !valid_name(name) {
        return Err("非法脚本名".into());
    }
    let p = script_dir().join(format!("{name}.js"));
    std::fs::read_to_string(&p).map_err(|e| format!("读取失败：{e}"))
}

pub fn script_save(name: &str, content: &str) -> Result<String, String> {
    if !valid_name(name) {
        return Err("非法脚本名（允许字母数字 ._ - 空格）".into());
    }
    let dir = script_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let p = dir.join(format!("{name}.js"));
    // 写路径过 guard（P1-1）：cases\ 不在工作区内，走「非检材」判定
    crate::guard::guard_write_or_err(&p)?;
    // 覆盖既有脚本前先快照（文档06 snapshots\）
    if p.is_file() {
        crate::guard::snapshot_file(&p)?;
    }
    std::fs::write(&p, content).map_err(|e| e.to_string())?;
    crate::audit::audit(
        "script_save",
        &p.display().to_string(),
        "done",
        "script-library",
        &format!("{} bytes", content.len()),
    );
    Ok(p.display().to_string())
}

pub fn script_delete(name: &str) -> Result<(), String> {
    if !valid_name(name) {
        return Err("非法脚本名".into());
    }
    let p = script_dir().join(format!("{name}.js"));
    if !p.is_file() {
        return Err(format!("脚本不存在：{name}.js"));
    }
    crate::guard::guard_write_or_err(&p)?;
    // 删除前快照（破坏性操作）
    crate::guard::snapshot_file(&p)?;
    std::fs::remove_file(&p).map_err(|e| e.to_string())?;
    crate::audit::audit("script_delete", name, "done", "script-library", "");
    Ok(())
}

// ---------------- AppProfile（U9：一次录好复用） ----------------

#[derive(Debug, Clone, Serialize)]
pub struct AppProfile {
    pub id: i64,
    pub case_name: String,
    pub package: String,
    pub uid: Option<i64>,
    pub apk_path: String,
    pub data_dirs: String,
    pub secret_files: String,
    pub secret_transform: String,
    pub entry_gesture: String,
    pub entry_coords: String,
    pub probe_targets: String,
    pub notes: String,
}

// 连接构造收敛到 store::open_db（批次⑪①）：此前自建连接缺 busy_timeout，
// 与 attach 落库并发写时瞬时 SQLITE_BUSY 直接报给用户
fn db() -> Result<Connection, String> {
    crate::store::open_db()
}

#[allow(clippy::too_many_arguments)] // 落库/编排函数的参数即字段清单（先例：brute_job_record）
pub fn profile_save(
    case_name: &str,
    id: Option<i64>,
    package: &str,
    uid: Option<i64>,
    apk_path: &str,
    data_dirs: &str,
    secret_files: &str,
    secret_transform: &str,
    entry_gesture: &str,
    entry_coords: &str,
    probe_targets: &str,
    notes: &str,
) -> Result<i64, String> {
    if package.trim().is_empty() {
        return Err("包名不能为空".into());
    }
    let conn = db()?;
    let case_id = crate::store::ensure_case_row(&conn, case_name)?;
    if let Some(id) = id {
        conn.execute(
            "UPDATE app_profiles SET package=?1, uid=?2, apk_path=?3, data_dirs=?4, secret_files=?5, secret_transform=?6, entry_gesture=?7, entry_coords=?8, probe_targets=?9, notes=?10 WHERE id=?11",
            rusqlite::params![package, uid, apk_path, data_dirs, secret_files, secret_transform, entry_gesture, entry_coords, probe_targets, notes, id],
        )
        .map_err(|e| e.to_string())?;
        crate::audit::audit("profile_update", package, "done", "profile-editor", "");
        Ok(id)
    } else {
        conn.execute(
            "INSERT INTO app_profiles(case_id, package, uid, apk_path, data_dirs, secret_files, secret_transform, entry_gesture, entry_coords, probe_targets, notes)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            rusqlite::params![case_id, package, uid, apk_path, data_dirs, secret_files, secret_transform, entry_gesture, entry_coords, probe_targets, notes],
        )
        .map_err(|e| e.to_string())?;
        let id = conn.last_insert_rowid();
        crate::audit::audit(
            "profile_create",
            package,
            "done",
            "profile-editor",
            case_name,
        );
        Ok(id)
    }
}

pub fn profile_list(case_name: &str) -> Result<Vec<AppProfile>, String> {
    let conn = db()?;
    let case_id = crate::store::ensure_case_row(&conn, case_name)?;
    let mut stmt = conn
        .prepare("SELECT id, package, uid, apk_path, data_dirs, secret_files, secret_transform, entry_gesture, entry_coords, probe_targets, notes FROM app_profiles WHERE case_id = ?1 ORDER BY id")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([case_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<i64>>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, String>(8)?,
                r.get::<_, String>(9)?,
                r.get::<_, String>(10)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        let (
            id,
            package,
            uid,
            apk_path,
            data_dirs,
            secret_files,
            secret_transform,
            entry_gesture,
            entry_coords,
            probe_targets,
            notes,
        ) = row.map_err(|e| e.to_string())?;
        out.push(AppProfile {
            id,
            case_name: case_name.into(),
            package,
            uid,
            apk_path,
            data_dirs,
            secret_files,
            secret_transform,
            entry_gesture,
            entry_coords,
            probe_targets,
            notes,
        });
    }
    Ok(out)
}

pub fn profile_delete(id: i64) -> Result<(), String> {
    let conn = db()?;
    conn.execute("DELETE FROM app_profiles WHERE id = ?1", [id])
        .map_err(|e| e.to_string())?;
    crate::audit::audit(
        "profile_delete",
        &id.to_string(),
        "done",
        "profile-editor",
        "",
    );
    Ok(())
}
