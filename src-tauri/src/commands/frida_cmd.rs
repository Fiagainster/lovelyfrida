use crate::backends::adb::AdbBackend;
use crate::services::experiment::{ExperimentConfig, ExperimentReport};
use crate::services::brute::{estimate as brute_estimate_fn, generate_c_skeleton, run_builtin as brute_run_fn, BruteEstimate, BruteResult};
use crate::services::crypto::{reconstruct as crypto_reconstruct_fn, ReconstructResult, Sample};
use crate::services::injection::{InjectionFile, InjectionReport};
use crate::services::extras_svc as ex;
use crate::services::ledger::{self as ledger_svc, EvidenceItem, Finding};
use crate::services::recorder::RecorderState;
use crate::services::session::{
    attach, detach, enumerate_processes, forward_setup, ping, server_install, server_status,
    FridaState, ProcEntry, SessionSnapshot, ServerStatusReport, StepReport,
};

/// frida 环境总览（版本三处一致 + 运行状态 + forward + 矩阵）
#[tauri::command]
pub async fn frida_server_status(
    state: tauri::State<'_, FridaState>,
) -> Result<ServerStatusReport, String> {
    let cfg = crate::config::get();
    Ok(server_status(&cfg, &state.channel).await)
}

/// 安装并启动设备端 frida-server（幂等，返回逐步报告）
#[tauri::command]
pub async fn frida_server_install(
    state: tauri::State<'_, FridaState>,
    recorder: tauri::State<'_, RecorderState>,
) -> Result<Vec<StepReport>, String> {
    let cfg = crate::config::get();
    let t0 = std::time::Instant::now();
    let result = server_install(&cfg, &state.channel).await;
    let ok = result.is_ok() && !result.as_ref().unwrap().iter().any(|s| s.status == "fail");
    crate::services::recorder::record_cmd(
        &recorder,
        "frida-server 安装并启动",
        "adb push <matrix>/frida-server /data/local/tmp/ && adb shell su -c 'chmod 755 … && nohup … &'",
        serde_json::json!({"port": cfg.frida_port}),
        if ok { "全部步骤通过".into() } else { "存在失败步骤（见逐步报告）".into() },
        t0.elapsed().as_millis() as u64,
    )
    .await;
    result
}

/// 建立 adb forward 并端到端验证
#[tauri::command]
pub async fn frida_forward_setup() -> Result<StepReport, String> {
    let cfg = crate::config::get();
    let adb = AdbBackend::detect(&cfg.adb_path, &cfg.doctor.adb_extra_paths).await?;
    let devices = adb.devices().await?;
    let serial = devices
        .iter()
        .find(|d| d.state == "device")
        .map(|d| d.serial.clone())
        .ok_or("无 device 状态设备")?;
    let info = forward_setup(&cfg, &adb, &serial).await?;
    Ok(info.step)
}

/// 进程/应用枚举（分组：system/user）
#[tauri::command]
pub async fn frida_processes(
    state: tauri::State<'_, FridaState>,
) -> Result<Vec<ProcEntry>, String> {
    let cfg = crate::config::get();
    enumerate_processes(&cfg, &state.channel).await
}

/// 附加到目标（pid 或进程名），走完整链路并等待 hello
#[tauri::command]
pub async fn frida_session_attach(
    app: tauri::AppHandle,
    state: tauri::State<'_, FridaState>,
    recorder: tauri::State<'_, RecorderState>,
    target: serde_json::Value,
) -> Result<SessionSnapshot, String> {
    let cfg = crate::config::get();
    let t0 = std::time::Instant::now();
    let result = attach(&app, &state, &cfg, target.clone()).await;
    let tgt = if target.is_u64() || target.is_i64() {
        format!("pid:{}", target.as_i64().unwrap_or(0))
    } else {
        target.as_str().unwrap_or("unknown").to_string()
    };
    crate::services::recorder::record_cmd(
        &recorder,
        "frida 附加会话",
        &format!("frida -H 127.0.0.1:<forward> -f/-n {tgt} -l core.js"),
        serde_json::json!({"target": tgt}),
        match &result {
            Ok(s) if s.phase == crate::services::session::SessionPhase::Running => {
                format!("运行中（session#{})", s.session_id.unwrap_or(0))
            }
            Ok(_) => "未到达 RUNNING".into(),
            Err(e) => format!("失败：{e}"),
        },
        t0.elapsed().as_millis() as u64,
    )
    .await;
    result
}

#[tauri::command]
pub async fn frida_session_detach(
    app: tauri::AppHandle,
    state: tauri::State<'_, FridaState>,
    recorder: tauri::State<'_, RecorderState>,
) -> Result<SessionSnapshot, String> {
    let t0 = std::time::Instant::now();
    let snap = detach(&app, &state).await?;
    crate::services::recorder::record_cmd(
        &recorder,
        "frida 分离会话",
        "卸载脚本 + session.detach()",
        serde_json::json!({}),
        "已分离".into(),
        t0.elapsed().as_millis() as u64,
    )
    .await;
    Ok(snap)
}

#[tauri::command]
pub async fn frida_session_status(
    state: tauri::State<'_, FridaState>,
) -> Result<SessionSnapshot, String> {
    Ok(state.session.lock().await.clone())
}

/// 消息回路自检：post ping（pong 走事件流到前端）
#[tauri::command]
pub async fn frida_session_ping(state: tauri::State<'_, FridaState>) -> Result<serde_json::Value, String> {
    ping(&state).await
}

/// 通用 RPC：调用 agent 的 rpc.exports 方法（探索器/探针/内存/REPL 全走这里）
#[tauri::command]
pub async fn frida_rpc(
    state: tauri::State<'_, FridaState>,
    f: String,
    args: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let s = state.session.lock().await;
    let Some(script_id) = s.script_id else {
        return Err("无活动脚本会话：先附加目标".into());
    };
    drop(s);
    let v = state
        .channel
        .call(
            "rpc_call",
            serde_json::json!({"script_id": script_id, "fn": f, "args": args}),
        )
        .await?;
    v.get("result")
        .cloned()
        .ok_or_else(|| "rpc 无返回值".to_string())
}

/// 数据回灌七步向导（文档04-C）
#[tauri::command]
pub async fn injection_run(
    recorder: tauri::State<'_, RecorderState>,
    package: String,
    files: Vec<InjectionFile>,
) -> Result<InjectionReport, String> {
    let cfg = crate::config::get();
    let t0 = std::time::Instant::now();
    let result = crate::services::injection::run(&cfg, &package, &files).await;
    crate::services::recorder::record_cmd(
        &recorder,
        "数据回灌",
        &format!(
            "adb push …/data/local/tmp && adb shell su -c 'cp/chown/restorecon/md5sum'（{} 个文件 → {}）",
            files.len(),
            package
        ),
        serde_json::json!({ "package": package, "files": files }),
        match &result {
            Ok(r) => format!("overall={}（{} 步）", r.overall, r.steps.len()),
            Err(e) => format!("失败：{e}"),
        },
        t0.elapsed().as_millis() as u64,
    )
    .await;
    result
}

/// 受控实验（文档04-E / U3）
#[tauri::command]
pub async fn experiment_run(
    state: tauri::State<'_, crate::services::session::FridaState>,
    exp: ExperimentConfig,
) -> Result<ExperimentReport, String> {
    let cfg = crate::config::get();
    crate::services::experiment::run(&cfg, &state.channel, Some(&state), exp).await
}

/// 算法还原（文档04-F / U4）：两组样本防假命中
#[tauri::command]
pub async fn crypto_reconstruct(samples: Vec<Sample>) -> Result<ReconstructResult, String> {
    Ok(crypto_reconstruct_fn(&samples))
}

/// 爆破预估三件套（文档04-G）
#[tauri::command]
pub async fn brute_estimate(scheme: crate::services::crypto::Scheme, mask: String) -> Result<BruteEstimate, String> {
    Ok(brute_estimate_fn(&mask, &scheme))
}

/// 内置爆破（小空间；★自测不过不许跑 C-07）
#[tauri::command]
pub async fn brute_run(
    recorder: tauri::State<'_, RecorderState>,
    scheme: crate::services::crypto::Scheme,
    mask: String,
    salt: String,
    known: Sample,
    max_candidates: Option<u64>,
) -> Result<BruteResult, String> {
    let t0 = std::time::Instant::now();
    let r = brute_run_fn(&scheme, &mask, &salt, &known, max_candidates.unwrap_or(5_000_000));
    crate::services::recorder::record_cmd(
        &recorder,
        "内置爆破",
        &format!("brute（mask={mask}, family={}）", scheme.family),
        serde_json::json!({ "mask": mask }),
        match &r.hit {
            Some(h) => format!("HIT pwd={h}"),
            None => format!("未命中（{}）", r.note),
        },
        t0.elapsed().as_millis() as u64,
    )
    .await;
    Ok(r)
}

/// 生成 C 专用爆破器骨架（无 hashcat 模式场景）
#[tauri::command]
pub async fn brute_generate_c(
    scheme: crate::services::crypto::Scheme,
    sample: Sample,
) -> Result<serde_json::Value, String> {
    let c = generate_c_skeleton(&scheme, &sample);
    let dir = {
        let cfg = crate::config::get();
        crate::paths::cases_root(&cfg).join("jobs")
    };
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let path = dir.join(format!("brute-{ts}.c"));
    std::fs::write(&path, c).map_err(|e| e.to_string())?;
    crate::audit::audit("brute_generate_c", &path.display().to_string(), "done", "restore-node", &scheme.family);
    Ok(serde_json::json!({ "path": path.display().to_string() }))
}

// ---------- M5：Evidence 台账 / 案卷包 ----------

#[tauri::command]
pub async fn ledger_add(
    case_name: String,
    question_id: String,
    question: String,
    answer: String,
    confidence: String,
    evidence: Vec<EvidenceItem>,
    source: String,
    screenshot_slot: String,
) -> Result<i64, String> {
    ledger_svc::add_finding(&case_name, &question_id, &question, &answer, &confidence, &evidence, &source, &screenshot_slot)
}

#[tauri::command]
pub async fn ledger_list(case_name: String) -> Result<Vec<Finding>, String> {
    ledger_svc::list_findings(&case_name)
}

#[tauri::command]
pub async fn ledger_delete(id: i64) -> Result<(), String> {
    ledger_svc::delete_finding(id)
}

#[tauri::command]
pub async fn ledger_export_md(case_name: String) -> Result<String, String> {
    ledger_svc::export_markdown(&case_name)
}

#[tauri::command]
pub async fn ledger_export_bundle(case_name: String) -> Result<String, String> {
    ledger_svc::export_bundle(&case_name)
}

// ---------- 能力包 B/D：脚本库 + AppProfile ----------

#[tauri::command]
pub async fn script_list() -> Result<Vec<ex::ScriptInfo>, String> { ex::script_list() }

#[tauri::command]
pub async fn script_read(name: String) -> Result<String, String> { ex::script_read(&name) }

#[tauri::command]
pub async fn script_save(name: String, content: String) -> Result<String, String> { ex::script_save(&name, &content) }

#[tauri::command]
pub async fn script_delete(name: String) -> Result<(), String> { ex::script_delete(&name) }

#[tauri::command]
#[allow(non_snake_case)]
pub async fn profile_save(
    caseName: String, id: Option<i64>, package: String, uid: Option<i64>,
    apkPath: String, dataDirs: String, secretFiles: String, secretTransform: String,
    entryGesture: String, entryCoords: String, probeTargets: String, notes: String,
) -> Result<i64, String> {
    ex::profile_save(&caseName, id, &package, uid, &apkPath, &dataDirs, &secretFiles,
        &secretTransform, &entryGesture, &entryCoords, &probeTargets, &notes)
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn profile_list(caseName: String) -> Result<Vec<ex::AppProfile>, String> {
    ex::profile_list(&caseName)
}

#[tauri::command]
pub async fn profile_delete(id: i64) -> Result<(), String> { ex::profile_delete(id) }

#[tauri::command]
pub async fn dumps_list() -> Result<Vec<serde_json::Value>, String> {
    let cfg = crate::config::get();
    let dir = crate::paths::cases_root(&cfg).join("dumps");
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            if let Ok(meta) = e.metadata() {
                out.push(serde_json::json!({
                    "name": e.file_name().to_string_lossy(),
                    "size": meta.len(),
                }));
            }
        }
    }
    Ok(out)
}
