use crate::backends::adb::AdbBackend;
use crate::config::AppConfig;
use serde::Serialize;
use std::time::Duration;

/// 环境体检（文档04-A）：动手前说清「这台机器能不能干活」。
/// 纪律：绿 = 有证据。每个状态灯由探测结果点亮，不能由命令返回码点亮（文档01 原则1）。
#[derive(Debug, Clone, Serialize)]
pub struct CheckResult {
    pub id: String,
    pub name: String,
    /// pass | warn | fail | skip（对应文档03 六态中的 ●/◑/✖/⊘）
    pub status: String,
    pub evidence: Vec<String>,
    pub fix: Option<String>,
    /// 对应诊断规则出处（文档05）
    pub rule: Option<String>,
    /// 等价命令（L2 命令层，可复制 / Alt+点击进终端）
    pub command: Option<String>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DoctorReport {
    pub checks: Vec<CheckResult>,
    pub overall: String,
    /// quick | deep
    pub mode: String,
    pub adb_path: String,
    pub adb_source: String,
    pub device_serial: Option<String>,
    pub duration_ms: u64,
}

/// 环境体检分两层（用户反馈：全量自检太慢）：
/// - quick（默认，<2s）：本地项 + adb 解析 + 设备现状读取，不主动发起 connect
/// - deep（兜底，手动触发）：对配置端口逐个 connect（15s 硬超时）+ offline 自愈曲线
pub async fn run(cfg: &AppConfig, deep: bool) -> DoctorReport {
    let t0 = std::time::Instant::now();
    let mode = if deep { "deep" } else { "quick" };

    let c1 = check_emulator(cfg).await;
    let c2 = check_adb(cfg).await;
    let (c3, backend, serial) = check_connect(cfg, deep).await;
    let c4 = check_root(&backend, &serial).await;
    let c5 = check_selinux(&backend, &serial).await;
    let c6 = check_abi(&backend, &serial).await;
    let c7 = check_frida_client().await;
    let c8 = check_version_matrix(&backend, &serial, &c7).await;
    let c9 = check_port_27042(&backend, &serial).await;
    let c10 = check_storage(cfg);
    let checks = vec![c1, c2, c3, c4, c5, c6, c7, c8, c9, c10];

    let overall = if checks.iter().any(|c| c.status == "fail") {
        "fail"
    } else if checks.iter().any(|c| c.status == "warn") {
        "warn"
    } else {
        "pass"
    };

    let (adb_path, adb_source) = match &backend {
        Some(b) => (b.path().display().to_string(), b.source.clone()),
        None => (String::new(), String::new()),
    };

    DoctorReport {
        checks,
        overall: overall.into(),
        mode: mode.into(),
        adb_path,
        adb_source,
        device_serial: serial,
        duration_ms: t0.elapsed().as_millis() as u64,
    }
}

fn result(id: &str, name: &str, status: &str, rule: &str, command: &str, evidence: Vec<String>, fix: Option<String>, ms: u64) -> CheckResult {
    CheckResult {
        id: id.into(),
        name: name.into(),
        status: status.into(),
        evidence,
        fix,
        rule: Some(rule.into()),
        command: Some(command.into()),
        duration_ms: ms,
    }
}

/// CHK-01 模拟器在跑：MuMu/Nemu 进程扫描 + 端口探测双证据（svchost 占端口不算数）。
async fn check_emulator(cfg: &AppConfig) -> CheckResult {
    let t0 = std::time::Instant::now();
    let mut evidence: Vec<String> = Vec::new();
    let ports = &cfg.doctor.emulator_ports;

    // 进程证据
    let proc_out = crate::backends::adb::quiet_command("tasklist")
        .args(["/FO", "CSV", "/NH"])
        .output()
        .await;
    let mut procs: Vec<String> = Vec::new();
    if let Ok(o) = proc_out {
        let text = String::from_utf8_lossy(&o.stdout).to_lowercase();
        for line in text.lines() {
            if line.contains("mumu") || line.contains("nemu") {
                if let Some(name) = line.split(',').next() {
                    procs.push(name.trim_matches('"').to_string());
                }
            }
        }
    }
    procs.sort();
    procs.dedup();
    if procs.is_empty() {
        evidence.push("进程扫描：未发现 MuMu/Nemu 相关进程".into());
    } else {
        evidence.push(format!("进程扫描：{}", procs.join(", ")));
    }

    // 端口证据（TCP 探测 + 记录归属）
    let mut listening: Vec<u16> = Vec::new();
    for p in ports {
        if tokio::net::TcpStream::connect(("127.0.0.1", *p))
            .await
            .is_ok()
        {
            listening.push(*p);
        }
    }
    evidence.push(format!(
        "端口探测 {:?}：{}",
        ports,
        if listening.is_empty() { "全部未监听".into() } else { format!("监听中 {listening:?}") }
    ));

    let status = if !procs.is_empty() {
        "pass"
    } else if !listening.is_empty() {
        evidence.push("⚠ 有端口监听但无 MuMu 进程（如 WinNAT 端口转发占用），不作为模拟器在跑的证据".into());
        "warn"
    } else {
        "fail"
    };
    let fix = if status == "fail" {
        Some("启动模拟器（MuMu/已配置的模拟器）；如端口不同，在设置中修改模拟器端口列表".into())
    } else {
        None
    };
    result(
        "CHK-01",
        "模拟器在跑",
        status,
        "E-01",
        "tasklist | findstr /I \"MuMu Nemu\"",
        evidence,
        fix,
        t0.elapsed().as_millis() as u64,
    )
}

/// CHK-02 找到可用 adb（E-04：模拟器自带优先，含用户自定义路径）。
async fn check_adb(cfg: &AppConfig) -> CheckResult {
    let t0 = std::time::Instant::now();
    match AdbBackend::detect(&cfg.adb_path, &cfg.doctor.adb_extra_paths).await {
        Ok(b) => {
            let mut evidence: Vec<String> = vec![format!("选用：{}（来源：{}）", b.path().display(), b.source)];
            for c in &b.candidates {
                evidence.push(format!(
                    "  [{}] {} — {}",
                    if c.exists { "存在" } else { "缺失" },
                    c.path,
                    c.source
                ));
            }
            result(
                "CHK-02",
                "找到可用 adb",
                "pass",
                "E-04",
                "adb version",
                evidence,
                None,
                t0.elapsed().as_millis() as u64,
            )
        }
        Err(e) => result(
            "CHK-02",
            "找到可用 adb",
            "fail",
            "E-04",
            "adb version",
            vec![e],
            Some("在 config.toml 配置 adb_path，或将模拟器安装到常见路径".into()),
            t0.elapsed().as_millis() as u64,
        ),
    }
}

/// CHK-03 adb 能连（E-02 15s 硬超时 / E-03 offline 自愈 / S-05 假绿灯二次确认）。
/// quick 模式只读设备现状（<1s）；deep 模式才主动 connect/自愈（兜底）。
/// 返回 backend 与已连接 serial 供后续检查复用。
async fn check_connect(cfg: &AppConfig, deep: bool) -> (CheckResult, Option<AdbBackend>, Option<String>) {
    let t0 = std::time::Instant::now();
    let backend = match AdbBackend::detect(&cfg.adb_path, &cfg.doctor.adb_extra_paths).await {
        Ok(b) => b,
        Err(e) => {
            return (
                result(
                    "CHK-03",
                    "adb 能连",
                    "fail",
                    "E-02/E-03/S-05",
                    "adb connect <host:port>",
                    vec![format!("前置失败：{e}")],
                    None,
                    t0.elapsed().as_millis() as u64,
                ),
                None,
                None,
            )
        }
    };

    // 已有 device 状态的设备 → 直接通过（USB 或已连接）
    let devices = backend.devices().await.unwrap_or_default();
    if let Some(d) = devices.iter().find(|d| d.state == "device") {
        let serial = d.serial.clone();
        return (
            result(
                "CHK-03",
                "adb 能连",
                "pass",
                "E-02/E-03/S-05",
                "adb devices",
                vec![
                    "已有 device 状态设备（无需 connect）".into(),
                    format!("serial = {serial}"),
                ],
                None,
                t0.elapsed().as_millis() as u64,
            ),
            Some(backend),
            Some(serial),
        );
    }

    // ---- quick 模式：到此为止，不主动 connect（慢操作留给深度体检兜底） ----
    if !deep {
        let snapshot = if devices.is_empty() { "空".to_string() } else {
            devices.iter().map(|d| format!("{}={}", d.serial, d.state)).collect::<Vec<_>>().join(", ")
        };
        let has_offline = devices.iter().any(|d| d.state == "offline");
        let hint = if has_offline {
            "存在 offline 残留记录：运行「深度体检」走自愈曲线（disconnect→2s→connect ×5）"
        } else {
            "快速体检不主动连接（避免多端口 15s 超时等待）；运行「深度体检」自动连接，或在下方设备连接面板手动连接"
        };
        return (
            result(
                "CHK-03",
                "adb 能连",
                "skip",
                "E-02/E-03/S-05",
                "adb devices",
                vec![format!("devices 快照：{snapshot}"), format!("⊘ {hint}")],
                Some("点击「深度体检」以自动连接".into()),
                t0.elapsed().as_millis() as u64,
            ),
            Some(backend),
            None,
        );
    }

    // ---- deep 模式：offline 残留优先走自愈，再逐端口 connect ----
    let offline: Vec<String> = devices
        .iter()
        .filter(|d| d.state == "offline" && d.serial.contains(':'))
        .map(|d| d.serial.clone())
        .collect();

    let timeout_s = cfg.doctor.deep_connect_timeout_s;

    // 端口候选：offline 残留优先（它说明模拟器曾在此端口），再试配置端口
    let mut ports: Vec<u16> = Vec::new();
    for s in &offline {
        if let Some(p) = s.rsplit(':').next().and_then(|x| x.parse::<u16>().ok()) {
            ports.push(p);
        }
    }
    for p in &cfg.doctor.emulator_ports {
        if !ports.contains(p) {
            ports.push(*p);
        }
    }

    let mut tried: Vec<String> = vec![format!(
        "devices 快照：{}",
        if devices.is_empty() { "空".into() } else {
            devices.iter().map(|d| format!("{}={}", d.serial, d.state)).collect::<Vec<_>>().join(", ")
        }
    )];
    for host_port in ports {
        let report = if offline.contains(&format!("127.0.0.1:{host_port}")) {
            backend.self_heal("127.0.0.1", host_port, timeout_s).await
        } else {
            backend.connect("127.0.0.1", host_port, timeout_s).await
        };
        for e in &report.evidence {
            tried.push(e.clone());
        }
        if report.ok {
            tried.push(format!("✔ 连接成功：{}", report.serial));
            return (
                result(
                    "CHK-03",
                    "adb 能连",
                    "pass",
                    "E-02/E-03/S-05",
                    "adb connect <host:port>",
                    tried,
                    None,
                    t0.elapsed().as_millis() as u64,
                ),
                Some(backend),
                Some(report.serial),
            );
        }
    }

    tried.push(format!(
        "✖ 尝试过全部端口 {} 均未连上（U1 要求：失败时列出试过端口与结果）",
        cfg.doctor.emulator_ports.iter().map(|p| p.to_string()).collect::<Vec<_>>().join("/")
    ));
    (
        result(
            "CHK-03",
            "adb 能连",
            "fail",
            "E-02/E-03/S-05",
            "adb connect <host:port>",
            tried,
            Some("确认模拟器已启动；若配置端口都失败，在 MuMu 设置中查看实例 adb 端口并更新设置里的端口列表".into()),
            t0.elapsed().as_millis() as u64,
        ),
        Some(backend),
        None,
    )
}

/// CHK-04 root（E-06）。无设备 → skip（⊘ 需说明理由）。
async fn check_root(backend: &Option<AdbBackend>, serial: &Option<String>) -> CheckResult {
    let t0 = std::time::Instant::now();
    let Some(b) = backend else {
        return skip("CHK-04", "root 权限", "前置 adb 不可用", t0);
    };
    let Some(serial) = serial else {
        return skip("CHK-04", "root 权限", "⊘ 无已连接设备（先完成 CHK-03）", t0);
    };
    let id_out = b.shell(serial, "id", Duration::from_secs(10)).await;
    match id_out {
        Ok(o) => {
            let text = format!("{}{}", o.stdout.trim(), o.stderr.trim());
            if text.contains("uid=0") {
                result(
                    "CHK-04",
                    "root 权限",
                    "pass",
                    "E-06",
                    "adb shell id",
                    vec![text],
                    None,
                    t0.elapsed().as_millis() as u64,
                )
            } else {
                // adb 非 root，试 su（文档：模拟器 su -c 路径，注意单引号 D-02）
                let su = b.shell(serial, "su -c id", Duration::from_secs(10)).await;
                let su_text = su
                    .map(|o| format!("{}{}", o.stdout.trim(), o.stderr.trim()))
                    .unwrap_or_default();
                if su_text.contains("uid=0") {
                    result(
                        "CHK-04",
                        "root 权限",
                        "pass",
                        "E-06",
                        "adb shell su -c id",
                        vec![format!("adb shell id → {text}"), format!("su -c id → {su_text}")],
                        Some("adb 非 root，但 su 可用：写操作将走 su -c（注意只能单引号，D-02）".into()),
                        t0.elapsed().as_millis() as u64,
                    )
                } else {
                    result(
                        "CHK-04",
                        "root 权限",
                        "fail",
                        "E-06",
                        "adb shell id",
                        vec![format!("id → {text}"), format!("su -c id → {su_text}")],
                        Some("MuMu：设置 → ROOT 权限开启；或 adb root 后重试".into()),
                        t0.elapsed().as_millis() as u64,
                    )
                }
            }
        }
        Err(e) => result(
            "CHK-04",
            "root 权限",
            "warn",
            "E-06",
            "adb shell id",
            vec![format!("执行失败：{e}")],
            None,
            t0.elapsed().as_millis() as u64,
        ),
    }
}

/// CHK-05 SELinux 模式（E-07）：Enforcing 不是失败，但回灌第⑥步必须 restorecon。
async fn check_selinux(backend: &Option<AdbBackend>, serial: &Option<String>) -> CheckResult {
    let t0 = std::time::Instant::now();
    let Some(b) = backend else {
        return skip("CHK-05", "SELinux 模式", "前置 adb 不可用", t0);
    };
    let Some(serial) = serial else {
        return skip("CHK-05", "SELinux 模式", "⊘ 无已连接设备", t0);
    };
    match b.shell(serial, "getenforce", Duration::from_secs(10)).await {
        Ok(o) => {
            let mode = o.stdout.trim().to_uppercase();
            let (status, fix) = match mode.as_str() {
                "PERMISSIVE" => ("pass", None),
                "ENFORCING" => (
                    "warn",
                    Some("Enforcing：数据回灌第⑥步必须 restorecon 刷标签（D-04，向导会自动执行）".to_string()),
                ),
                _ => ("warn", Some("getenforce 输出无法识别，SELinux 状态未知".to_string())),
            };
            result(
                "CHK-05",
                "SELinux 模式",
                status,
                "E-07",
                "adb shell getenforce",
                vec![format!("getenforce → {}", if mode.is_empty() { "（空）" } else { &mode })],
                fix,
                t0.elapsed().as_millis() as u64,
            )
        }
        Err(e) => result(
            "CHK-05",
            "SELinux 模式",
            "warn",
            "E-07",
            "adb shell getenforce",
            vec![format!("执行失败：{e}")],
            None,
            t0.elapsed().as_millis() as u64,
        ),
    }
}

/// CHK-06 目标 ABI（E-08）：x86_64 为本轮目标（D4），x86/armeabi 只提示不预置。
async fn check_abi(backend: &Option<AdbBackend>, serial: &Option<String>) -> CheckResult {
    let t0 = std::time::Instant::now();
    let Some(b) = backend else {
        return skip("CHK-06", "目标 ABI", "前置 adb 不可用", t0);
    };
    let Some(serial) = serial else {
        return skip("CHK-06", "目标 ABI", "⊘ 无已连接设备", t0);
    };
    match b
        .shell(serial, "getprop ro.product.cpu.abi", Duration::from_secs(10))
        .await
    {
        Ok(o) => {
            let abi = o.stdout.trim().to_string();
            let (status, fix) = match abi.as_str() {
                "x86_64" | "arm64-v8a" => ("pass", None),
                "" => ("warn", Some("getprop 无输出，ABI 未知".to_string())),
                _ => (
                    "warn",
                    Some(format!("ABI={abi}：x86/armeabi 只提示不预置 frida-server（文档02 兼容矩阵）")),
                ),
            };
            result(
                "CHK-06",
                "目标 ABI",
                status,
                "E-08",
                "adb shell getprop ro.product.cpu.abi",
                vec![format!("ro.product.cpu.abi = {}", if abi.is_empty() { "（空）" } else { &abi })],
                fix,
                t0.elapsed().as_millis() as u64,
            )
        }
        Err(e) => result(
            "CHK-06",
            "目标 ABI",
            "warn",
            "E-08",
            "adb shell getprop ro.product.cpu.abi",
            vec![format!("执行失败：{e}")],
            None,
            t0.elapsed().as_millis() as u64,
        ),
    }
}

/// CHK-07 本机 frida 客户端版本（S-01 三处一致之一）。
async fn check_frida_client() -> CheckResult {
    let t0 = std::time::Instant::now();
    match crate::backends::adb::run_raw(Path::new("frida"), &["--version"], Duration::from_secs(10)).await {
        Ok(o) if !o.timed_out && !o.stdout.trim().is_empty() => {
            let v = o.stdout.trim().to_string();
            result(
                "CHK-07",
                "本机 frida 客户端版本",
                "pass",
                "S-01",
                "frida --version",
                vec![format!("frida 客户端 = {v}")],
                None,
                t0.elapsed().as_millis() as u64,
            )
        }
        _ => result(
            "CHK-07",
            "本机 frida 客户端版本",
            "fail",
            "S-01",
            "frida --version",
            vec!["PATH 上未找到 frida CLI 或执行超时".into()],
            Some("pip install frida（M1 将内置 wheel 矩阵 + 一键安装）".into()),
            t0.elapsed().as_millis() as u64,
        ),
    }
}

/// CHK-08 三处版本一致性（S-01）：客户端 vs bin 矩阵 vs 设备端。
async fn check_version_matrix(
    backend: &Option<AdbBackend>,
    serial: &Option<String>,
    client_check: &CheckResult,
) -> CheckResult {
    let t0 = std::time::Instant::now();
    let matrix = crate::paths::frida_server_matrix_dir();
    let mut versions: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&matrix) {
        for e in entries.flatten() {
            if e.path().is_dir() {
                versions.push(e.file_name().to_string_lossy().to_string());
            }
        }
    }
    versions.sort();
    let client_version = client_check
        .evidence
        .first()
        .and_then(|s| s.rsplit([' ', '=']).next().map(|s| s.to_string()))
        .unwrap_or_default();

    let mut evidence: Vec<String> = vec![
        format!("① 客户端：{client_version}"),
        format!(
            "② bin\\frida-server 矩阵：{}",
            if versions.is_empty() { "空（M1 下载）".into() } else { versions.join(", ") }
        ),
    ];

    // 设备端：已推送的 frida-server 文件
    let device_leg;
    let mut device_ok = false;
    if let (Some(b), Some(s)) = (backend, serial) {
        let out = b
            .shell(s, "su -c 'ls /data/local/tmp'", Duration::from_secs(10))
            .await;
        match out {
            Ok(o) => {
                let files: Vec<String> = o
                    .stdout
                    .lines()
                    .map(|l| l.trim().to_string())
                    .filter(|l| l.contains("frida"))
                    .collect();
                device_leg = format!(
                    "③ 设备端 /data/local/tmp：{}",
                    if files.is_empty() { "无 frida 相关文件".into() } else { files.join(", ") }
                );
                device_ok = !files.is_empty();
            }
            Err(e) => device_leg = format!("③ 设备端：检查失败（{e}）"),
        }
    } else {
        device_leg = "③ 设备端：⊘ 无已连接设备".into();
    }
    evidence.push(device_leg);

    let matrix_has_client = !client_version.is_empty() && versions.iter().any(|v| v == &client_version);
    let (status, fix) = if client_version.is_empty() {
        ("warn", Some("先解决 CHK-07（客户端缺失）".into()))
    } else if !matrix_has_client {
        (
            "warn",
            Some(format!("bin\\frida-server 矩阵缺 {client_version}：M1 提供「推送匹配版」一键下载推送")),
        )
    } else if !device_ok {
        (
            "warn",
            Some("设备端尚未推送 frida-server：M1 会话链路将自动推送匹配版本".into()),
        )
    } else if let (Some(b), Some(s)) = (backend, serial) {
        // 设备端运行版本实测（阶段③：不再用 M1 占位文案）——server 同款探测
        let ver_out = b
            .shell(
                s,
                "su -c '/data/local/tmp/frida-server --version' 2>/dev/null || echo unknown",
                Duration::from_secs(15),
            )
            .await;
        let running = ver_out
            .ok()
            .map(|o| {
                o.stdout
                    .trim()
                    .trim_end_matches("unknown")
                    .trim()
                    .to_string()
            })
            .filter(|v| !v.is_empty());
        match running {
            Some(v) => {
                evidence.push(format!("③½ 设备端运行版本实测：{v}"));
                if v == client_version {
                    ("pass", None)
                } else {
                    (
                        "warn",
                        Some(format!(
                            "设备端运行 {v} ≠ 客户端 {client_version}（S-01）：会话链路将 pkill 后推送匹配版"
                        )),
                    )
                }
            }
            None => (
                "warn",
                Some("设备端文件已存在但未运行（--version 无输出）：会话链路将自动启动匹配版本".into()),
            ),
        }
    } else {
        ("warn", Some("无设备可实测运行版本".into()))
    };
    result(
        "CHK-08",
        "三处版本一致性",
        status,
        "S-01",
        "adb shell ls /data/local/tmp",
        evidence,
        fix,
        t0.elapsed().as_millis() as u64,
    )
}

/// CHK-09 27042 端口：设备端实测监听（S-02/S-05）+ 本机占用（S-06）。
async fn check_port_27042(backend: &Option<AdbBackend>, serial: &Option<String>) -> CheckResult {
    let t0 = std::time::Instant::now();
    let mut evidence: Vec<String> = Vec::new();
    let mut device_leg_done = false;

    if let (Some(b), Some(s)) = (backend, serial) {
        let out = b
            .shell(s, "ss -tlnp 2>/dev/null | grep 27042 || echo NO_LISTENER", Duration::from_secs(10))
            .await;
        match out {
            Ok(o) => {
                let text = o.stdout.trim().to_string();
                if text.contains("27042") && !text.contains("NO_LISTENER") {
                    evidence.push(format!("✔ 设备端 27042 实测监听中：{text}"));
                } else {
                    evidence.push("✖ 设备端 27042 未监听（frida-server 未启动，M1 会话链路负责启动）".into());
                }
                device_leg_done = true;
            }
            Err(e) => evidence.push(format!("设备端检查失败：{e}")),
        }
    }

    // 本机 27042（S-06：被占自动换 27043）
    let host_occupied = tokio::net::TcpStream::connect(("127.0.0.1", 27042))
        .await
        .is_ok();
    if host_occupied {
        evidence.push("⚠ 本机 127.0.0.1:27042 已被占用（S-06：forward 时将自动改用 27043）".into());
    } else {
        evidence.push("本机 27042 空闲".into());
    }

    let status = if !device_leg_done {
        "skip"
    } else if evidence.iter().any(|e| e.starts_with("✔")) {
        if host_occupied { "warn" } else { "pass" }
    } else if evidence.iter().any(|e| e.starts_with("✖")) {
        "warn"
    } else {
        "warn"
    };
    let fix = if evidence.iter().any(|e| e.starts_with("✖")) {
        Some("frida-server 未启动：M1 提供「启动并验证监听」动作（假绿灯以 ss -tlnp 为准，S-05）".into())
    } else {
        None
    };
    result(
        "CHK-09",
        "27042 端口",
        status,
        "S-02/S-05/S-06",
        "adb shell \"ss -tlnp | grep 27042\"",
        evidence,
        fix,
        t0.elapsed().as_millis() as u64,
    )
}

/// CHK-10 存储与权限（07 工程纪律：不写系统盘）。
fn check_storage(cfg: &AppConfig) -> CheckResult {
    let t0 = std::time::Instant::now();
    let mut evidence: Vec<String> = Vec::new();
    let mut ok = true;

    let workspace = crate::paths::workspace_root(cfg);
    match std::fs::create_dir_all(&workspace) {
        Ok(()) => {
            let probe = workspace.join(".write-test");
            match std::fs::write(&probe, b"ok").and_then(|_| std::fs::remove_file(&probe)) {
                Ok(()) => evidence.push(format!("✔ 工作区可写：{}", workspace.display())),
                Err(e) => {
                    ok = false;
                    evidence.push(format!("✖ 工作区写测试失败：{e}"));
                }
            }
        }
        Err(e) => {
            ok = false;
            evidence.push(format!("✖ 工作区创建失败：{}（{e}）", workspace.display()));
        }
    }

    // 磁盘余量
    let root = crate::paths::app_root();
    let free_gb = fs2::available_space(&root)
        .map(|b| b as f64 / 1024.0 / 1024.0 / 1024.0)
        .ok();
    match free_gb {
        Some(gb) => {
            evidence.push(format!(
                "{} 剩余 {gb:.1} GB",
                root.display().to_string().chars().take(3).collect::<String>()
            ));
            if gb < 1.0 {
                ok = false;
            }
        }
        None => evidence.push("磁盘余量探测失败（不阻断）".into()),
    }
    if cfg.read_only_roots.is_empty() {
        evidence.push("⚠ read_only_roots 为空：尚未登记检材只读根（在设置中添加后 Workspace Guard 生效更完整）".into());
    }

    let status = if ok { "pass" } else { "fail" };
    result(
        "CHK-10",
        "存储与权限",
        status,
        "07章-工程纪律",
        "—",
        evidence,
        (!ok).then(|| "检查磁盘空间与工作区权限".to_string()),
        t0.elapsed().as_millis() as u64,
    )
}

fn skip(id: &str, name: &str, reason: &str, t0: std::time::Instant) -> CheckResult {
    result(
        id,
        name,
        "skip",
        "—",
        "—",
        vec![reason.to_string()],
        None,
        t0.elapsed().as_millis() as u64,
    )
}

use std::path::Path;
