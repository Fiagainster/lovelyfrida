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

/// 进程/应用枚举（分组：system/user）；通道B 失败时按 P3-4 降级 frida-ps 解析
#[tauri::command]
pub async fn frida_processes(
    state: tauri::State<'_, FridaState>,
) -> Result<Vec<ProcEntry>, String> {
    let cfg = crate::config::get();
    match enumerate_processes(&cfg, &state.channel).await {
        Ok(p) => Ok(p),
        Err(be) => {
            if !crate::backends::frida_c::FridaChannelC::available().await {
                return Err(be);
            }
            let host_port = state
                .session
                .lock()
                .await
                .forward_host_port
                .unwrap_or(cfg.frida_port);
            let rows = crate::backends::frida_c::enumerate_processes_cli("127.0.0.1", host_port)
                .await
                .map_err(|e| format!("{be}；通道C 兜底也失败：{e}"))?;
            tracing::info!("[通道C] 进程枚举兜底成功（{} 项）", rows.len());
            Ok(crate::services::session::proc_entries_from_pairs(rows))
        }
    }
}

/// 附加到目标（pid 或进程名），走完整链路并等待 hello；caseName 用于会话落库（P2-3）
#[tauri::command]
#[allow(non_snake_case)]
pub async fn frida_session_attach(
    app: tauri::AppHandle,
    state: tauri::State<'_, FridaState>,
    recorder: tauri::State<'_, RecorderState>,
    target: serde_json::Value,
    caseName: Option<String>,
) -> Result<SessionSnapshot, String> {
    let cfg = crate::config::get();
    let t0 = std::time::Instant::now();
    let result = attach(&app, &state, &cfg, target.clone(), caseName).await;
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
    // 通道C（CLI 兜底）不支持交互式 RPC：诚实报错，不假装（文档10 P3-4 能力边界）
    if s.channel == "C" {
        return Err(format!(
            "通道 C（CLI 兜底）不支持交互式 RPC（{f}）：探针/探索器/REPL 需要通道 B（安装 Python + frida，或在设置中选通道 B）"
        ));
    }
    let Some(script_id) = s.script_id else {
        return Err("无活动脚本会话：先附加目标".into());
    };
    drop(s);
    // 审计（P1-4）：任意 agent RPC 是"揭示明文"级动作；入参截断存储，audit 内部再脱敏
    let args_short: String = args.to_string().chars().take(200).collect();
    let r = state
        .channel
        .call(
            "rpc_call",
            serde_json::json!({"script_id": script_id, "fn": f, "args": args}),
        )
        .await;
    match &r {
        Ok(_) => crate::audit::audit("frida_rpc", &f, "done", "agent-rpc", &args_short),
        Err(e) => crate::audit::audit("frida_rpc", &f, "fail", "agent-rpc", &e),
    }
    let v = r?;
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

/// 受控实验（文档04-E / U3）；caseName 用于实验落库（P2-3）
#[tauri::command]
#[allow(non_snake_case)]
pub async fn experiment_run(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::services::session::FridaState>,
    exp: ExperimentConfig,
    caseName: Option<String>,
) -> Result<ExperimentReport, String> {
    let cfg = crate::config::get();
    crate::services::experiment::run(&cfg, &state.channel, &app, Some(&state), exp, caseName).await
}

/// 算法还原（文档04-F / U4）：两组样本防假命中。穷举是 CPU 密集，放阻塞线程池；
/// 还原出的方案落 crypto_schemes 表（P2-3）。
#[tauri::command]
#[allow(non_snake_case)]
pub async fn crypto_reconstruct(
    samples: Vec<Sample>,
    caseName: Option<String>,
) -> Result<ReconstructResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let r = crypto_reconstruct_fn(&samples);
        if let Some(scheme) = &r.scheme {
            let refs = serde_json::json!(
                samples.iter().map(|s| s.target.clone()).collect::<Vec<_>>()
            )
            .to_string();
            crate::store::crypto_scheme_record(
                caseName.as_deref().unwrap_or("默认案件"),
                &scheme.family,
                &scheme.concat,
                &scheme.salt_form,
                &scheme.chain_input,
                scheme.iterations,
                &scheme.output_encoding,
                r.self_test_passed,
                &refs,
            );
        }
        r
    })
    .await
    .map_err(|e| format!("后台任务失败：{e}"))
    .map(Ok)?
}

/// frida-server 按需下载（C1）：version=None → sidecar 客户端版本（S-01 三处一致）；
/// abis=None → manifest 登记的全部 ABI。sha256 对账通过才落 workspace/frida-server/。
#[tauri::command]
pub async fn frida_server_fetch(
    state: tauri::State<'_, crate::services::session::FridaState>,
    version: Option<String>,
    abis: Option<Vec<String>>,
) -> Result<crate::services::frida_fetch::FetchReport, String> {
    let version = match version {
        Some(v) if !v.trim().is_empty() => v,
        _ => {
            let hello = state.channel.call("hello", serde_json::json!({})).await?;
            hello
                .get("frida")
                .and_then(|s| s.as_str())
                .ok_or("sidecar 未返回 frida 版本（无法确定客户端版本）")?
                .to_string()
        }
    };
    let cfg = crate::config::get();
    crate::services::frida_fetch::fetch(&cfg, version, abis).await
}

/// 爆破预估三件套（文档04-G）
#[tauri::command]
pub async fn brute_estimate(scheme: crate::services::crypto::Scheme, mask: String) -> Result<BruteEstimate, String> {
    Ok(brute_estimate_fn(&mask, &scheme))
}

/// 内置爆破（小空间；★自测不过不许跑 C-07）。DFS 穷举 CPU 密集，放阻塞线程池；
/// 作业结果落 brute_jobs 表（P2-3）。
#[tauri::command]
#[allow(non_snake_case)]
pub async fn brute_run(
    recorder: tauri::State<'_, RecorderState>,
    scheme: crate::services::crypto::Scheme,
    mask: String,
    salt: String,
    known: Sample,
    max_candidates: Option<u64>,
    caseName: Option<String>,
) -> Result<BruteResult, String> {
    let t0 = std::time::Instant::now();
    let mask_display = mask.clone();
    let family_display = scheme.family.clone();
    let r = tauri::async_runtime::spawn_blocking(move || {
        let r = brute_run_fn(&scheme, &mask, &salt, &known, max_candidates.unwrap_or(5_000_000));
        // 作业落库（失败不阻断：库不可用时爆破结果仍返回 UI）
        let sets = crate::services::brute::expand_mask(&mask);
        let total: u64 = sets.iter().fold(1u64, |acc, s| acc.saturating_mul(s.len() as u64));
        let space = serde_json::json!({ "mask": mask, "total": total }).to_string();
        let speed = if r.duration_ms > 0 {
            r.tried as f64 / (r.duration_ms as f64 / 1000.0)
        } else {
            0.0
        };
        crate::store::brute_job_record(
            caseName.as_deref().unwrap_or("默认案件"),
            &space,
            total,
            speed,
            "builtin",
            r.self_test_passed,
            if r.hit.is_some() { "hit" } else { "exhausted" },
            r.hit.as_deref().unwrap_or(""),
            r.hit.is_some(),
            &r.note,
        );
        r
    })
    .await
    .map_err(|e| format!("后台任务失败：{e}"))?;
    crate::services::recorder::record_cmd(
        &recorder,
        "内置爆破",
        &format!("brute（mask={mask_display}, family={family_display}）"),
        serde_json::json!({ "mask": mask_display }),
        match &r.hit {
            Some(h) => format!("HIT pwd={h}"),
            None => format!("未命中（{}）", r.note),
        },
        t0.elapsed().as_millis() as u64,
    )
    .await;
    Ok(r)
}

/// 生成 C 专用爆破器骨架（无 hashcat 模式场景；掩码循环 + OpenMP + 强制自测）
#[tauri::command]
pub async fn brute_generate_c(
    scheme: crate::services::crypto::Scheme,
    sample: Sample,
    mask: String,
) -> Result<serde_json::Value, String> {
    if mask.trim().is_empty() {
        return Err("掩码为空：先填爆破掩码（如 ?u?l?l?d?d?d?d?d?d）".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let c = generate_c_skeleton(&scheme, &sample, &mask);
        let dir = {
            let cfg = crate::config::get();
            crate::paths::cases_root(&cfg).join("jobs")
        };
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let path = dir.join(format!("brute-{ts}.c"));
        // 写路径过 guard（P1-1）
        crate::guard::guard_write_or_err(&path)?;
        std::fs::write(&path, c).map_err(|e| e.to_string())?;
        crate::audit::audit("brute_generate_c", &path.display().to_string(), "done", "restore-node", &scheme.family);
        Ok(serde_json::json!({ "path": path.display().to_string() }))
    })
    .await
    .map_err(|e| format!("后台任务失败：{e}"))?
}

// ---------- M5：Evidence 台账 / 案卷包 ----------
// SQLite 同步驱动（rusqlite）：统一 spawn_blocking，避免阻塞 tokio worker（P1-5）

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
    tauri::async_runtime::spawn_blocking(move || {
        ledger_svc::add_finding(&case_name, &question_id, &question, &answer, &confidence, &evidence, &source, &screenshot_slot)
    })
    .await
    .map_err(|e| format!("后台任务失败：{e}"))?
}

#[tauri::command]
pub async fn ledger_list(case_name: String) -> Result<Vec<Finding>, String> {
    tauri::async_runtime::spawn_blocking(move || ledger_svc::list_findings(&case_name))
        .await
        .map_err(|e| format!("后台任务失败：{e}"))?
}

#[tauri::command]
pub async fn ledger_delete(id: i64) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || ledger_svc::delete_finding(id))
        .await
        .map_err(|e| format!("后台任务失败：{e}"))?
}

#[tauri::command]
pub async fn ledger_export_md(case_name: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || ledger_svc::export_markdown(&case_name))
        .await
        .map_err(|e| format!("后台任务失败：{e}"))?
}

#[tauri::command]
pub async fn ledger_export_bundle(case_name: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || ledger_svc::export_bundle(&case_name))
        .await
        .map_err(|e| format!("后台任务失败：{e}"))?
}

// ---------- 能力包 B/D：脚本库 + AppProfile ----------

#[tauri::command]
pub async fn script_list() -> Result<Vec<ex::ScriptInfo>, String> {
    tauri::async_runtime::spawn_blocking(ex::script_list)
        .await
        .map_err(|e| format!("后台任务失败：{e}"))?
}

#[tauri::command]
pub async fn script_read(name: String) -> Result<String, String> {
    let audited_name = name.clone();
    let r = tauri::async_runtime::spawn_blocking(move || ex::script_read(&name))
        .await
        .map_err(|e| format!("后台任务失败：{e}"))?;
    // 审计（P1-4）：脚本读取留痕（内容本身不入账）
    crate::audit::audit(
        "script_read",
        &audited_name,
        if r.is_ok() { "done" } else { "fail" },
        "script-library",
        "",
    );
    r
}

#[tauri::command]
pub async fn script_save(name: String, content: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || ex::script_save(&name, &content))
        .await
        .map_err(|e| format!("后台任务失败：{e}"))?
}

#[tauri::command]
pub async fn script_delete(name: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || ex::script_delete(&name))
        .await
        .map_err(|e| format!("后台任务失败：{e}"))?
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn profile_save(
    caseName: String, id: Option<i64>, package: String, uid: Option<i64>,
    apkPath: String, dataDirs: String, secretFiles: String, secretTransform: String,
    entryGesture: String, entryCoords: String, probeTargets: String, notes: String,
) -> Result<i64, String> {
    tauri::async_runtime::spawn_blocking(move || {
        ex::profile_save(&caseName, id, &package, uid, &apkPath, &dataDirs, &secretFiles,
            &secretTransform, &entryGesture, &entryCoords, &probeTargets, &notes)
    })
    .await
    .map_err(|e| format!("后台任务失败：{e}"))?
}

#[tauri::command]
#[allow(non_snake_case)]
pub async fn profile_list(caseName: String) -> Result<Vec<ex::AppProfile>, String> {
    tauri::async_runtime::spawn_blocking(move || ex::profile_list(&caseName))
        .await
        .map_err(|e| format!("后台任务失败：{e}"))?
}

#[tauri::command]
pub async fn profile_delete(id: i64) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || ex::profile_delete(id))
        .await
        .map_err(|e| format!("后台任务失败：{e}"))?
}

#[tauri::command]
pub async fn dumps_list() -> Result<Vec<serde_json::Value>, String> {
    tauri::async_runtime::spawn_blocking(|| {
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
    })
    .await
    .map_err(|e| format!("后台任务失败：{e}"))?
}

// ---------- B2 落库数据面（文档10）：历史查询 ----------

#[tauri::command]
pub async fn history_sessions(limit: Option<i64>) -> Result<Vec<crate::services::history::SessionRow>, String> {
    tauri::async_runtime::spawn_blocking(move || crate::services::history::list_sessions(limit.unwrap_or(20)))
        .await
        .map_err(|e| format!("后台任务失败：{e}"))?
}

#[tauri::command]
pub async fn history_runs(limit: Option<i64>) -> Result<Vec<crate::services::history::RunRow>, String> {
    tauri::async_runtime::spawn_blocking(move || crate::services::history::list_runs(limit.unwrap_or(20)))
        .await
        .map_err(|e| format!("后台任务失败：{e}"))?
}

#[tauri::command]
pub async fn history_experiments(limit: Option<i64>) -> Result<Vec<crate::services::history::ExperimentRow>, String> {
    tauri::async_runtime::spawn_blocking(move || crate::services::history::list_experiments(limit.unwrap_or(20)))
        .await
        .map_err(|e| format!("后台任务失败：{e}"))?
}

#[tauri::command]
pub async fn history_brute_jobs(limit: Option<i64>) -> Result<Vec<crate::services::history::BruteJobRow>, String> {
    tauri::async_runtime::spawn_blocking(move || crate::services::history::list_brute_jobs(limit.unwrap_or(20)))
        .await
        .map_err(|e| format!("后台任务失败：{e}"))?
}
