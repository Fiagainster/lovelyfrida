//! 数据回灌向导（文档04-C，七步逐条状态灯）：
//! ①停应用（不可跳过 D-06）②空跑建档（D-07）③建目录 ④推送（失败降级 /data/local/tmp 中转 + su cp，D-01）
//! ⑤改属主（uid 实测自动取）⑥刷标签（restorecon，Enforcing 时必做 D-04）⑦md5 逐文件校验。
//! 第④步后扫描 shared_prefs 登录态键并预警（O-03 联动）。
use crate::backends::adb::{AdbBackend, ProcOutput};
use crate::config::AppConfig;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InjectionFile {
    pub local_path: String,
    pub device_dir: String,
    pub device_name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct StepReport {
    pub name: String,
    pub status: String, // pass | warn | fail | skip
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InjectionReport {
    pub steps: Vec<StepReport>,
    pub login_state_warnings: Vec<String>,
    pub overall: String,
}

fn md5_hex(data: &[u8]) -> String {
    use md5::{Digest, Md5};
    let mut h = Md5::new();
    h.update(data);
    hex::encode(h.finalize())
}

async fn shell(adb: &AdbBackend, serial: &str, cmd: &str) -> Result<ProcOutput, String> {
    adb.shell(serial, cmd, Duration::from_secs(60)).await
}

pub async fn run(
    cfg: &AppConfig,
    package: &str,
    files: &[InjectionFile],
) -> Result<InjectionReport, String> {
    use crate::services::device_shell::{self, sq, su_c};
    if !device_shell::valid_package(package) {
        return Err(format!(
            "包名不合法：{package}（仅允许字母/数字/点/下划线）"
        ));
    }
    for f in files {
        if !device_shell::valid_filename(&f.device_name) {
            return Err(format!(
                "文件名不合法：{}（不得含路径分隔符）",
                f.device_name
            ));
        }
        if !f.device_dir.starts_with('/') {
            return Err(format!("目标目录必须是设备绝对路径：{}", f.device_dir));
        }
    }
    let adb = AdbBackend::detect(&cfg.adb_path, &cfg.doctor.adb_extra_paths).await?;
    let devices = adb.devices().await?;
    let Some(serial) = devices
        .iter()
        .find(|d| d.state == "device")
        .map(|d| d.serial.clone())
    else {
        return Err("无 device 状态设备".into());
    };

    let mut steps: Vec<StepReport> = Vec::new();
    let mut login_warnings: Vec<String> = Vec::new();

    // ① 停应用（D-06：不可跳过）
    let stop = shell(&adb, &serial, &format!("am force-stop {package}")).await;
    match stop {
        Ok(_) => steps.push(StepReport {
            name: "① 停应用".into(),
            status: "pass".into(),
            evidence: vec![format!("am force-stop {package}")],
        }),
        Err(e) => {
            steps.push(StepReport {
                name: "① 停应用".into(),
                status: "fail".into(),
                evidence: vec![e],
            });
            return Ok(finish(steps, login_warnings));
        }
    }

    // ② 空跑建档（D-07：让 App 自己把私有目录建好）
    let launch = shell(
        &adb,
        &serial,
        &format!("monkey -p {package} -c android.intent.category.LAUNCHER 1"),
    )
    .await;
    match launch {
        Ok(o) => {
            tokio::time::sleep(Duration::from_secs(3)).await;
            let _ = shell(&adb, &serial, &format!("am force-stop {package}")).await;
            steps.push(StepReport {
                name: "② 空跑建档".into(),
                status: "pass".into(),
                evidence: vec![
                    "monkey 启动一次后 force-stop（让 App 建好自己的目录）".into(),
                    o.stdout.trim().to_string(),
                ],
            });
        }
        Err(e) => {
            steps.push(StepReport {
                name: "② 空跑建档".into(),
                status: "warn".into(),
                evidence: vec![format!("空跑失败（继续，但目录可能不存在）：{e}")],
            });
        }
    }

    // ③ 建目录（逐条 mkdir -p）
    let mut dir_ok = true;
    let mut dir_evidence: Vec<String> = Vec::new();
    for f in files {
        let dir = &f.device_dir;
        let r = shell(&adb, &serial, &su_c(&format!("mkdir -p {}", sq(dir)))).await;
        match r {
            Ok(_) => dir_evidence.push(format!("mkdir -p {dir} ✓")),
            Err(e) => {
                dir_ok = false;
                dir_evidence.push(format!("mkdir -p {dir} ✖ {e}"));
            }
        }
    }
    steps.push(StepReport {
        name: "③ 建目录".into(),
        status: if dir_ok { "pass".into() } else { "fail".into() },
        evidence: dir_evidence,
    });

    // ④ 推送（先尝试中转 + su cp；目标目录可写时直接 push 也可行——统一走中转更稳，D-01/D-02）
    let mut push_ok = true;
    let mut push_evidence: Vec<String> = Vec::new();
    let mut pushed: Vec<(String, String)> = Vec::new(); // (device_abs, local)
    for f in files {
        let name = &f.device_name;
        let local = &f.local_path;
        let target = format!("{}/{}", f.device_dir.trim_end_matches('/'), name);
        let tmp = format!("/data/local/tmp/lf-inject-{name}");
        if !std::path::Path::new(local).is_file() {
            push_ok = false;
            push_evidence.push(format!("✖ 本地文件不存在：{local}"));
            continue;
        }
        match adb
            .push(&serial, local, &tmp, Duration::from_secs(300))
            .await
        {
            Ok(o) if !o.timed_out => {
                let cp = shell(
                    &adb,
                    &serial,
                    &su_c(&format!(
                        "cp {} {} && rm -f {}",
                        sq(&tmp),
                        sq(&target),
                        sq(&tmp)
                    )),
                )
                .await;
                match cp {
                    Ok(_) => {
                        push_evidence.push(format!("✓ {local} → {tmp} → {target}"));
                        pushed.push((target, local.clone()));
                    }
                    Err(e) => {
                        push_ok = false;
                        push_evidence.push(format!(
                            "✖ cp 到 {target} 失败：{e}（注意 su 只能单引号，D-02）"
                        ));
                    }
                }
            }
            Ok(_) => {
                push_ok = false;
                push_evidence.push(format!("✖ push 超时：{local}"));
            }
            Err(e) => {
                push_ok = false;
                push_evidence.push(format!("✖ push 失败：{e}"));
            }
        }
    }
    steps.push(StepReport {
        name: "④ 推送".into(),
        status: if push_ok {
            "pass".into()
        } else {
            "fail".into()
        },
        evidence: push_evidence,
    });
    if !push_ok {
        return Ok(finish(steps, login_warnings));
    }

    // ④.5 登录态键预警（O-03 联动）：扫描 shared_prefs 里的敏感键
    let prefs = shell(
        &adb,
        &serial,
        &format!("su -c 'ls /data/data/{package}/shared_prefs/ 2>/dev/null'"),
    )
    .await;
    if let Ok(o) = prefs {
        let pref_files: Vec<&str> = o
            .stdout
            .lines()
            .filter(|l| l.trim().ends_with(".xml"))
            .collect();
        for pf in pref_files.iter().take(10) {
            if let Ok(content) = shell(
                &adb,
                &serial,
                // pf 来自设备 ls 输出，视为不可信输入（恶意 App 可造出带引号的 prefs 文件名）
                &su_c(&format!("cat /data/data/{package}/shared_prefs/{}", sq(pf))),
            )
            .await
            {
                let lower = content.stdout.to_lowercase();
                for key in [
                    "password", "token", "login", "session", "auth", "islogin", "logged",
                ] {
                    if lower.contains(key) {
                        login_warnings.push(format!(
                            "{} 含登录态键「{}」——灌数据后 App 可能带着旧登录态直接进主页，探针零命中（O-03）",
                            pf, key
                        ));
                        break;
                    }
                }
            }
        }
    }

    // ⑤ 改属主（uid 实测自动取）
    let uid_out = shell(
        &adb,
        &serial,
        &format!("su -c 'stat -c %u /data/data/{package}'"),
    )
    .await;
    match uid_out {
        Ok(o) => {
            let uid = o.stdout.trim().to_string();
            if uid.is_empty() || !uid.chars().all(|c| c.is_ascii_digit()) {
                steps.push(StepReport {
                    name: "⑤ 改属主".into(),
                    status: "warn".into(),
                    evidence: vec![format!(
                        "uid 获取异常（{}），跳过 chown——如 App 闪退请手动检查（D-03）",
                        uid
                    )],
                });
            } else {
                let mut ev = vec![format!("uid = {uid}（实测）")];
                let mut all_ok = true;
                for (target, _) in &pushed {
                    let r = shell(
                        &adb,
                        &serial,
                        // uid 已在上方按纯数字校验；target 含路径/文件名，走 sq 引用
                        &su_c(&format!("chown {uid}:{uid} {}", sq(target))),
                    )
                    .await;
                    match r {
                        Ok(_) => ev.push(format!("chown {uid}:{uid} {target} ✓")),
                        Err(e) => {
                            all_ok = false;
                            ev.push(format!("chown {target} ✖ {e}"));
                        }
                    }
                }
                steps.push(StepReport {
                    name: "⑤ 改属主".into(),
                    status: if all_ok { "pass".into() } else { "fail".into() },
                    evidence: ev,
                });
            }
        }
        Err(e) => steps.push(StepReport {
            name: "⑤ 改属主".into(),
            status: "warn".into(),
            evidence: vec![format!("uid 获取失败：{e}")],
        }),
    }

    // ⑥ 刷标签（restorecon；Enforcing 时必做，D-04：not found 非阻断）
    let enforce = shell(&adb, &serial, "getenforce").await;
    let enforcing = enforce
        .map(|o| o.stdout.trim().eq_ignore_ascii_case("enforcing"))
        .unwrap_or(false);
    if enforcing {
        let mut ev = vec!["SELinux=Enforcing，必须 restorecon".into()];
        let mut ok = true;
        for (target, _) in &pushed {
            let r = shell(
                &adb,
                &serial,
                &su_c(&format!("restorecon {} 2>/dev/null; echo done", sq(target))),
            )
            .await;
            match r {
                Ok(o) if o.stdout.contains("done") => ev.push(format!("restorecon {target} ✓")),
                Ok(o) => {
                    ev.push(format!("restorecon {target}：{}", o.stdout.trim()));
                    if o.stderr.contains("not found") {
                        ev.push("（restorecon 不存在，非阻断，D-04）".into());
                    }
                }
                Err(e) => {
                    ok = false;
                    ev.push(format!("restorecon {target} ✖ {e}"));
                }
            }
        }
        steps.push(StepReport {
            name: "⑥ 刷标签".into(),
            status: if ok { "pass".into() } else { "warn".into() },
            evidence: ev,
        });
    } else {
        steps.push(StepReport {
            name: "⑥ 刷标签".into(),
            status: "skip".into(),
            evidence: vec!["⊘ SELinux 非 Enforcing（或未知），无需 restorecon".into()],
        });
    }

    // ⑦ md5 逐文件校验
    let mut md5_evidence: Vec<String> = Vec::new();
    let mut md5_ok = true;
    for (target, local) in &pushed {
        // 大文件整读是重 IO，放阻塞线程池（P1-5）
        let local_path = local.clone();
        let local_bytes = tauri::async_runtime::spawn_blocking(move || {
            std::fs::read(&local_path).unwrap_or_default()
        })
        .await
        .unwrap_or_default();
        let local_md5 = md5_hex(&local_bytes);
        let remote = shell(&adb, &serial, &su_c(&format!("md5sum {}", sq(target)))).await;
        match remote {
            Ok(o) => {
                let remote_md5 = o.stdout.split_whitespace().next().unwrap_or("").to_string();
                if remote_md5.eq_ignore_ascii_case(&local_md5) {
                    md5_evidence.push(format!("✓ {target} md5={local_md5}"));
                } else {
                    md5_ok = false;
                    md5_evidence.push(format!(
                        "✖ {target} md5 不一致：本地 {local_md5} / 设备 {remote_md5}"
                    ));
                }
            }
            Err(e) => {
                md5_ok = false;
                md5_evidence.push(format!("✖ {target} md5sum 执行失败：{e}"));
            }
        }
    }
    steps.push(StepReport {
        name: "⑦ md5 校验".into(),
        status: if md5_ok { "pass".into() } else { "fail".into() },
        evidence: md5_evidence,
    });

    crate::audit::audit(
        "injection_run",
        package,
        "done",
        "injection-wizard",
        &format!("{} 文件", files.len()),
    );
    Ok(finish(steps, login_warnings))
}

fn finish(steps: Vec<StepReport>, warnings: Vec<String>) -> InjectionReport {
    let overall = if steps.iter().any(|s| s.status == "fail") {
        "fail"
    } else if steps.iter().any(|s| s.status == "warn") || !warnings.is_empty() {
        "warn"
    } else {
        "pass"
    };
    InjectionReport {
        steps,
        login_state_warnings: warnings,
        overall: overall.into(),
    }
}
