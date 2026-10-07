//! 受控实验台（文档04-E / U3）：变量 = 设备上的文件内容；观测通道 = 指定探针的命中参数。
//! 每组输入：写文件 → 停应用 → 重启 → 取新 pid → 重新 attach + 加载 agent + 重挂探针
//! （force-stop 会杀掉脚本会话，插桩必须重建）→ 采集窗口内命中 → 观测记录。
//! 差分矩阵（行=输入组，列=观测通道，变异度排序）由前端计算渲染。
use crate::backends::adb::AdbBackend;
use crate::config::AppConfig;
use crate::services::injection::InjectionFile;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExperimentTemplate {
    pub name: String,
    pub content: String, // 文本内容（writeup 场景：password.json 是 16 字节文本）
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExperimentProbe {
    pub clazz: String,
    pub method: String,
    #[serde(default = "default_max_len")]
    pub max_len: u32,
    #[serde(default)]
    pub capture_ret: bool,
}

fn default_max_len() -> u32 {
    128
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExperimentConfig {
    pub package: String,
    pub device_file_dir: String,
    pub device_file_name: String,
    /// 观测探针声明（每组重启后需要重新挂载）
    pub probe: ExperimentProbe,
    pub wait_s: u64,
    pub templates: Vec<ExperimentTemplate>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Observation {
    pub group: String,
    pub hits: u64,
    /// 第一条命中的 args（typed JSON）
    pub args: Option<Value>,
    pub ret: Option<Value>,
    pub wall: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExperimentReport {
    pub experiment_id: String,
    pub observations: Vec<Observation>,
    pub errors: Vec<String>,
}

/// 写入单文件（回灌链的精简版：push → su cp → chown → restorecon）
async fn write_device_file(
    adb: &AdbBackend,
    serial: &str,
    package: &str,
    f: &InjectionFile,
    content: &[u8],
) -> Result<(), String> {
    use crate::services::device_shell::{sq, su_c};
    // device_name 会拼进本机 %TEMP% 文件名，必须先拦下宿主侧穿越（批次⑩）
    if !crate::services::device_shell::valid_filename(&f.device_name) {
        return Err(format!("文件名不合法：{}（不得含路径分隔符）", f.device_name));
    }
    if !f.device_dir.starts_with('/') {
        return Err(format!("目标目录必须是设备绝对路径：{}", f.device_dir));
    }
    let local_tmp = std::env::temp_dir().join(format!("lf-exp-{}", f.device_name));
    std::fs::write(&local_tmp, content).map_err(|e| format!("写本地临时文件失败：{e}"))?;
    let target = format!("{}/{}", f.device_dir.trim_end_matches('/'), f.device_name);
    let tmp = format!("/data/local/tmp/lf-exp-{}", f.device_name);
    let local_tmp_str = local_tmp.to_str().ok_or("本机临时目录路径非 UTF-8")?;
    adb.push(serial, local_tmp_str, &tmp, Duration::from_secs(120))
        .await
        .map_err(|e| format!("push 失败：{e}"))?;
    let uid_out = adb
        .shell(
            serial,
            &format!("su -c 'stat -c %u /data/data/{package}'"),
            Duration::from_secs(15),
        )
        .await?;
    let uid = uid_out.stdout.trim().to_string();
    adb.shell(
        serial,
        &su_c(&format!(
            "cp {a} {b} && chown {uid}:{uid} {b} && restorecon {b} 2>/dev/null; rm -f {a}; echo ok",
            a = sq(&tmp),
            b = sq(&target)
        )),
        Duration::from_secs(30),
    )
    .await
    .map_err(|e| format!("cp/chown 失败：{e}"))?;
    let _ = std::fs::remove_file(&local_tmp);
    Ok(())
}

pub async fn run(
    cfg: &AppConfig,
    frida: &crate::backends::frida::FridaChannelB,
    app: &tauri::AppHandle,
    session_state: Option<&crate::services::session::FridaState>,
    exp: ExperimentConfig,
    case_name: Option<String>,
) -> Result<ExperimentReport, String> {
    if !crate::services::device_shell::valid_package(&exp.package) {
        return Err(format!(
            "包名不合法：{}（仅允许字母/数字/点/下划线）",
            exp.package
        ));
    }
    let adb = AdbBackend::detect(&cfg.adb_path, &cfg.doctor.adb_extra_paths).await?;
    let devices = adb.devices().await?;
    let serial = devices
        .iter()
        .find(|d| d.state == "device")
        .map(|d| d.serial.clone())
        .ok_or("无 device 状态设备")?;

    let experiment_id = chrono::Local::now().format("%Y%m%d-%H%M%S%.3f").to_string();
    let mut observations: Vec<Observation> = Vec::new();
    let mut errors: Vec<String> = Vec::new();

    for tpl in &exp.templates {
        // 1) 写变量文件
        let f = InjectionFile {
            local_path: String::new(),
            device_dir: exp.device_file_dir.clone(),
            device_name: exp.device_file_name.clone(),
        };
        if let Err(e) = write_device_file(&adb, &serial, &exp.package, &f, tpl.content.as_bytes()).await {
            errors.push(format!("[{}] 写文件失败：{e}", tpl.name));
            continue;
        }

        // 2) 停应用（清场；脚本会话随之消失）
        let _ = adb
            .shell(&serial, &format!("am force-stop {}", exp.package), Duration::from_secs(15))
            .await;
        tokio::time::sleep(Duration::from_millis(800)).await;

        // 3) forward（幂等，S-06 主机端口探测）
        let fwd = crate::services::session::forward_setup(cfg, &adb, &serial).await?;
        let conn = frida
            .call("remote_connect", json!({"host": "127.0.0.1", "port": fwd.host_port}))
            .await?;
        let device = conn
            .get("key")
            .and_then(|k| k.as_str())
            .ok_or("remote_connect 未返回 device key")?
            .to_string();

        // 订阅必须在 resume 之前（broadcast 不回放；spawn 挂起期装好探针后一 resume 命中就开始）
        let mut rx = frida.subscribe().await?;

        // 4) spawn（挂起）→ attach → 加载 agent → 重挂探针 → resume
        //    这保证探针在 App 任何代码执行之前就位（文档04-B / writeup 标准模式）
        let spawn = frida
            .call("spawn", json!({"device": device, "program": exp.package}))
            .await
            .map_err(|e| format!("[{}] spawn 失败：{e}", tpl.name))?;
        let spawn_pid = spawn.get("pid").and_then(|v| v.as_u64()).unwrap_or(0);
        let (session_id, script_id) =
            crate::backends::frida::attach_and_load_core(frida, "127.0.0.1", fwd.host_port, json!(spawn_pid))
                .await
                .map_err(|e| format!("[{}] spawn 后附加失败：{e}", tpl.name))?;
        let add = frida
            .call(
                "rpc_call",
                json!({
                    "script_id": script_id,
                    "fn": "addProbes",
                    "args": [{ "probes": [{
                        "id": format!("exp-{}", tpl.name),
                        "clazz": exp.probe.clazz,
                        "method": exp.probe.method,
                        "maxLen": exp.probe.max_len,
                        "captureRet": exp.probe.capture_ret,
                    }]}]
                }),
            )
            .await?;
        let probe_status = add
            .pointer("/result/results/0/status")
            .or_else(|| add.pointer("/results/0/status"))
            .and_then(|v| v.as_str())
            .unwrap_or("active（未解析，按成功处理）")
            .to_string();
        if probe_status != "active" && probe_status != "waiting" {
            errors.push(format!(
                "[{}] 探针重挂状态异常：{probe_status}（类可能未加载，P-05）",
                tpl.name
            ));
        }
        frida
            .call("resume", json!({"device": device, "pid": spawn_pid}))
            .await
            .map_err(|e| format!("[{}] resume 失败：{e}", tpl.name))?;

        // 5) 采集 wait_s 秒内的 probe_hit
        let deadline = tokio::time::Instant::now() + Duration::from_secs(exp.wait_s.max(2));
        let mut hits: u64 = 0;
        let mut first_args: Option<Value> = None;
        let mut first_ret: Option<Value> = None;
        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_millis(300), rx.recv()).await {
                Err(_) => continue,
                // Lagged 只是本订阅者落后被丢事件，channel 仍活着：继续采集。
                // 按 break 处理会把事件洪峰误判成窗口结束（hits 偏低且无提示）。
                Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(n))) => {
                    tracing::warn!("[experiment {}] 事件积压丢弃 {n} 条（窗口继续）", tpl.name);
                    continue;
                }
                Ok(Err(_)) => break,
                Ok(Ok(ev)) => {
                    if let crate::backends::frida::FridaEvent::Message {
                        script_id: sid,
                        kind,
                        payload: Some(p),
                        ..
                    } = ev
                    {
                        if sid != script_id || kind != "send" {
                            continue;
                        }
                        if p.get("t").and_then(|v| v.as_str()) == Some("probe_hit") {
                            hits += 1;
                            if first_args.is_none() {
                                first_args = p.get("args").cloned();
                                first_ret = p.get("ret").cloned();
                            }
                        }
                    }
                }
            }
        }
        observations.push(Observation {
            group: tpl.name.clone(),
            hits,
            args: first_args,
            ret: first_ret,
            wall: chrono::Local::now().format("%H:%M:%S").to_string(),
        });

        // 会话状态更新为本次 spawn 的会话（UI 保持真实）——此前只改不发事件，
        // 前端状态灯要等下一次轮询才刷新
        if let Some(st) = session_state {
            let snap = {
                let mut g = st.session.lock().await;
                g.session_id = Some(session_id);
                g.script_id = Some(script_id);
                g.target = Some(format!("pid:{spawn_pid}"));
                g.phase = crate::services::session::SessionPhase::Running;
                crate::services::session::push_ev(&mut g, format!(
                    "[实验 {}] spawn pid={spawn_pid} → 探针 {probe_status} → 命中 {hits}",
                    tpl.name
                ));
                g.updated_at = chrono::Local::now().format("%H:%M:%S%.3f").to_string();
                g.clone()
            };
            use tauri::Emitter;
            let _ = app.emit("session-state", snap);
        }
    }

    // 落盘实验记录（cases/experiments/<id>.json）
    let dir = {
        let c = crate::config::get();
        crate::paths::cases_root(&c).join("experiments")
    };
    let _ = std::fs::create_dir_all(&dir);
    let record = json!({
        "id": experiment_id,
        "config": exp,
        "observations": observations,
        "errors": errors,
    });
    let path = dir.join(format!("exp-{experiment_id}.json"));
    // 写路径过 guard（P1-1）
    let report_json = serde_json::to_string_pretty(&record).unwrap_or_default();
    if let Err(e) = crate::guard::guard_write_or_err(&path) {
        tracing::warn!("[experiment] 实验记录落盘被拒绝：{e}");
    } else if !report_json.is_empty() {
        let _ = std::fs::write(path, report_json.clone());
    }
    // 实验记录落库（P2-3）：experiments + experiment_cases；失败不阻断
    {
        let case = case_name.unwrap_or_else(|| "默认案件".into());
        let title = format!("{}（{}）", exp.package, exp.probe.method);
        let cases_json = serde_json::to_string(&exp.templates).unwrap_or_else(|_| "[]".into());
        let report_clone = report_json.clone();
        let _ = tauri::async_runtime::spawn_blocking(move || {
            crate::store::experiment_record(&case, &title, &report_clone, &cases_json)
        })
        .await;
    }
    crate::audit::audit(
        "experiment_run",
        &exp.package,
        "done",
        "experiment-bench",
        &format!("{} 组", exp.templates.len()),
    );

    Ok(ExperimentReport {
        experiment_id,
        observations,
        errors,
    })
}
