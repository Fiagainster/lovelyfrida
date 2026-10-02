//! Evidence 台账（文档04-H / M5）：发现登记 + 置信度约束 + 导出。
//! 约束（文档06，原则5 落进数据层）：confidence=high 必须同时有 math 与 device 证据。
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub id: i64,
    pub case_id: i64,
    pub question_id: String,
    pub question: String,
    pub answer: String,
    pub confidence: String, // high | medium | low
    pub evidence: Vec<EvidenceItem>,
    pub source: String,
    pub screenshot_slot: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceItem {
    pub kind: String, // math | device
    pub note: String,
}

fn db() -> Result<Connection, String> {
    let conn = Connection::open(crate::paths::cases_db_path())
        .map_err(|e| format!("打开 cases.db 失败：{e}"))?;
    conn.pragma_update(None, "foreign_keys", "ON").map_err(|e| e.to_string())?;
    Ok(conn)
}

fn ensure_case(conn: &Connection, case_name: &str) -> Result<i64, String> {
    let mut stmt = conn
        .prepare("SELECT id FROM cases WHERE name = ?1")
        .map_err(|e| e.to_string())?;
    let existing: Option<i64> = stmt
        .query_row([case_name], |r| r.get(0))
        .map(Some)
        .or_else(|e| if e == rusqlite::Error::QueryReturnedNoRows { Ok(None) } else { Err(e) })
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

/// 新增发现。★ high 置信度必须双证据（math + device）——把原则5变成代码约束。
pub fn add_finding(
    case_name: &str,
    question_id: &str,
    question: &str,
    answer: &str,
    confidence: &str,
    evidence: &[EvidenceItem],
    source: &str,
    screenshot_slot: &str,
) -> Result<i64, String> {
    if question.trim().is_empty() || answer.trim().is_empty() {
        return Err("问题与答案不能为空".into());
    }
    let has_math = evidence.iter().any(|e| e.kind == "math");
    let has_device = evidence.iter().any(|e| e.kind == "device");
    if confidence == "high" && !(has_math && has_device) {
        return Err(
            "置信度 high 必须同时具备 math（数学自证）与 device（真机复现）两条证据（原则5，不允许默默升格）"
                .into(),
        );
    }
    if !matches!(confidence, "high" | "medium" | "low") {
        return Err(format!("未知置信度：{confidence}"));
    }
    let conn = db()?;
    let case_id = ensure_case(&conn, case_name)?;
    let ev_json = serde_json::to_string(evidence).map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO findings(case_id, question_id, question, answer, confidence, evidence, source, screenshot_slot)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![case_id, question_id, question, answer, confidence, ev_json, source, screenshot_slot],
    )
    .map_err(|e| e.to_string())?;
    let id = conn.last_insert_rowid();
    crate::audit::audit("finding_add", question_id, "done", "ledger", &format!("confidence={confidence}"));
    Ok(id)
}

pub fn list_findings(case_name: &str) -> Result<Vec<Finding>, String> {
    let conn = db()?;
    let case_id = ensure_case(&conn, case_name)?;
    let mut stmt = conn
        .prepare("SELECT id, question_id, question, answer, confidence, evidence, source, screenshot_slot FROM findings WHERE case_id = ?1 ORDER BY id")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([case_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
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
        let (id, qid, q, a, conf, ev, src, slot) = row.map_err(|e| e.to_string())?;
        let evidence: Vec<EvidenceItem> = serde_json::from_str(&ev).unwrap_or_default();
        out.push(Finding {
            id,
            case_id,
            question_id: qid,
            question: q,
            answer: a,
            confidence: conf,
            evidence,
            source: src,
            screenshot_slot: slot,
        });
    }
    Ok(out)
}

pub fn delete_finding(id: i64) -> Result<(), String> {
    let conn = db()?;
    conn.execute("DELETE FROM findings WHERE id = ?1", [id])
        .map_err(|e| e.to_string())?;
    crate::audit::audit("finding_delete", &id.to_string(), "done", "ledger", "用户删除");
    Ok(())
}

/// 导出 Markdown（Obsidian 友好，带【截图位 N-x】编号，文档04-H / U8）
pub fn export_markdown(case_name: &str) -> Result<String, String> {
    let findings = list_findings(case_name)?;
    if findings.is_empty() {
        return Err("该案暂无发现记录".into());
    }
    let mut md = format!("# {case_name} · 发现台账\n\n> 由 LovelyFrida Evidence Ledger 导出。置信度：high=数学自证+真机复现双证据。\n\n");
    for (i, f) in findings.iter().enumerate() {
        let ev: Vec<String> = f.evidence.iter().map(|e| format!("{}({})", e.kind, e.note)).collect();
        md.push_str(&format!(
            "## {}. 【{}】 {}\n\n- **问题**：{}\n- **答案**：`{}`\n- **置信度**：{}（证据：{}）\n- **出处**：{}\n- 【截图位 {}】\n\n",
            i + 1,
            f.question_id,
            f.question,
            f.question,
            f.answer,
            f.confidence,
            ev.join("、"),
            f.source,
            f.screenshot_slot
        ));
    }
    let dir = {
        let cfg = crate::config::get();
        crate::paths::cases_root(&cfg).join("exports")
    };
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let path = dir.join(format!("{case_name}-findings-{ts}.md"));
    // 写路径过 guard（P1-1）：导出目标不得落在检材只读根
    crate::guard::guard_write_or_err(&path)?;
    std::fs::write(&path, md).map_err(|e| e.to_string())?;
    crate::audit::audit("ledger_export_md", &path.display().to_string(), "done", "ledger", case_name);
    Ok(path.display().to_string())
}

/// 案卷包（U10）：Case + findings(md) + traces + experiments + jobs 归拢到一个可交接目录
pub fn export_bundle(case_name: &str) -> Result<String, String> {
    let cfg = crate::config::get();
    let cases_root = crate::paths::cases_root(&cfg);
    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let bundle = cases_root.join("exports").join(format!("{case_name}-bundle-{ts}"));
    // 写路径过 guard（P1-1）
    crate::guard::guard_write_or_err(&bundle)?;
    std::fs::create_dir_all(&bundle).map_err(|e| e.to_string())?;

    // findings.md
    let md = export_markdown(case_name)?;
    let md_path = std::path::Path::new(&md);
    std::fs::copy(md_path, bundle.join("findings.md")).map_err(|e| e.to_string())?;

    // traces / experiments / jobs 整目录拷贝
    for sub in ["traces", "experiments", "jobs"] {
        let src = cases_root.join(sub);
        if src.is_dir() {
            copy_dir(&src, &bundle.join(sub))?;
        }
    }

    // README（接手者按此复现，U10）
    let readme = format!(
        "# 案卷包 · {case_name}\n\n生成时间：{}\n\n## 内容\n- findings.md：发现台账（含截图位编号）\n- traces/：探针命中 trace（jsonl）\n- experiments/：受控实验记录\n- jobs/：爆破作业与 C 骨架\n\n## 复现方式\n接手者按 findings.md 的出处与等价命令，在 LovelyFrida 中重新附加目标并重放；\n每条 high 置信度发现均满足 数学自证 + 真机复现 双证据。\n",
        chrono::Local::now().to_rfc3339()
    );
    std::fs::write(bundle.join("README.md"), readme).map_err(|e| e.to_string())?;
    crate::audit::audit("ledger_export_bundle", &bundle.display().to_string(), "done", "ledger", case_name);
    Ok(bundle.display().to_string())
}

fn copy_dir(src: &std::path::Path, dst: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    for entry in std::fs::read_dir(src).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir(&from, &to)?;
        } else {
            std::fs::copy(&from, &to).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// 序列化辅助（供导出 json 用）
#[allow(dead_code)] // M5 后续 json 导出使用
pub fn findings_to_json(findings: &[Finding]) -> Value {
    serde_json::to_value(findings).unwrap_or(Value::Null)
}
