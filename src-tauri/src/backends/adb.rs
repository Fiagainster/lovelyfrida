use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// 进程输出（三层事件的 L3 原料；Doctor 用于产生证据）。
#[derive(Debug, Clone, Serialize)]
pub struct ProcOutput {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AdbDevice {
    pub serial: String,
    pub state: String,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AdbCandidate {
    pub path: String,
    pub exists: bool,
    pub chosen: bool,
    pub source: String,
}

/// 连接报告：把「试过什么、每步结果」全量带回 UI（U1：失败列出试过端口与结果）。
#[derive(Debug, Clone, Serialize)]
pub struct ConnectReport {
    pub ok: bool,
    pub serial: String,
    pub state: String,
    pub attempts: u32,
    pub evidence: Vec<String>,
    pub elapsed_ms: u64,
}

#[derive(Clone)]
pub struct AdbBackend {
    adb: PathBuf,
    pub source: String,
    pub candidates: Vec<AdbCandidate>,
}

// detect 全量扫描（注册表 reg query ×5 + 盘符目录扫描 + where + adb version 实测）
// 一次要 spawn 十几个子进程；而候选集在进程生命周期内基本不变。
// 按 (configured, extra_paths) 为键缓存结果（5 分钟 TTL），消除每次命令级调用的重复扫描。
impl AdbBackend {
    /// adb 解析顺序（E-04：模拟器自带优先）：config > env > 用户自定义 > MuMu 扫描 > bin\adb > PATH。
    /// 带 5 分钟缓存（键 = 配置输入；改设置即换键，不受陈旧缓存影响）。
    pub async fn detect(configured: &str, extra_paths: &[String]) -> Result<Self, String> {
        use std::collections::HashMap;
        use std::time::Instant;
        static CACHE: std::sync::OnceLock<
            tokio::sync::Mutex<HashMap<String, (AdbBackend, Instant)>>,
        > = std::sync::OnceLock::new();
        let cache = CACHE.get_or_init(|| tokio::sync::Mutex::new(HashMap::new()));
        let key = format!("{}\u{1}{}", configured, extra_paths.join("\u{1}"));
        {
            let map = cache.lock().await;
            if let Some((backend, at)) = map.get(&key) {
                if at.elapsed() < Duration::from_secs(300) {
                    return Ok(backend.clone());
                }
            }
        }
        let backend = Self::detect_uncached(configured, extra_paths).await?;
        cache
            .lock()
            .await
            .insert(key, (backend.clone(), Instant::now()));
        Ok(backend)
    }

    async fn detect_uncached(configured: &str, extra_paths: &[String]) -> Result<Self, String> {
        let mut candidates: Vec<AdbCandidate> = Vec::new();

        let configured = configured.trim();
        if !configured.is_empty() {
            candidates.push(AdbCandidate {
                path: configured.to_string(),
                exists: Path::new(configured).is_file(),
                chosen: false,
                source: "config.toml".into(),
            });
        }
        if let Ok(env_path) = std::env::var("LOVELYFRIDA_MUMU_ADB") {
            if !env_path.trim().is_empty() {
                candidates.push(AdbCandidate {
                    path: env_path.trim().to_string(),
                    exists: Path::new(env_path.trim()).is_file(),
                    chosen: false,
                    source: "env LOVELYFRIDA_MUMU_ADB".into(),
                });
            }
        }
        for p in extra_paths {
            let t = p.trim();
            if t.is_empty() {
                continue;
            }
            candidates.push(AdbCandidate {
                path: t.to_string(),
                exists: Path::new(t).is_file(),
                chosen: false,
                source: "设置·自定义 adb".into(),
            });
        }
        for p in scan_mumu_adb().await {
            candidates.push(AdbCandidate {
                path: p.display().to_string(),
                exists: true,
                chosen: false,
                source: "MuMu 安装目录扫描".into(),
            });
        }
        // PATH 解析为绝对路径（供 server 亲和匹配；`where adb` 首行）
        if let Ok(o) = quiet_command("where")
            .arg("adb")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output()
            .await
        {
            if let Some(first) = String::from_utf8_lossy(&o.stdout).lines().next() {
                let p = first.trim();
                if !p.is_empty() && Path::new(p).is_file() {
                    candidates.push(AdbCandidate {
                        path: p.to_string(),
                        exists: true,
                        chosen: false,
                        source: "PATH（绝对路径）".into(),
                    });
                }
            }
        }
        let bundled = crate::paths::bundled_adb_path();
        candidates.push(AdbCandidate {
            path: bundled.display().to_string(),
            exists: bundled.is_file(),
            chosen: false,
            source: "bin\\adb（随包）".into(),
        });

        // server 亲和（E-01/E-04 变体）：多个 adb 二进制版本不一致时会互相 kill/restart
        // 对方的 server（实测一次安装链因此拖到 4 分钟）。若已有 server 在运行，
        // 优先选用同一个二进制。
        if let Some(server_path) = query_running_server_path().await {
            let idx = candidates
                .iter()
                .position(|c| c.exists && c.path.eq_ignore_ascii_case(&server_path));
            if let Some(i) = idx {
                for c in candidates.iter_mut() {
                    c.chosen = false;
                }
                candidates[i].chosen = true;
                candidates[i].source = format!(
                    "{}（与运行中 server 相同，避免重启乒乓）",
                    candidates[i].source
                );
            }
        }

        // PATH 兜底：`adb` 能执行即可（放在最后）
        if !candidates.iter().any(|c| c.exists) {
            if let Ok(o) = run_raw(Path::new("adb"), &["version"], Duration::from_secs(5)).await {
                if !o.timed_out && o.stdout.contains("Android Debug Bridge") {
                    candidates.push(AdbCandidate {
                        path: "adb".into(),
                        exists: true,
                        chosen: false,
                        source: "PATH".into(),
                    });
                }
            }
        }

        let chosen_idx = candidates
            .iter()
            .position(|c| c.chosen && c.exists)
            .or_else(|| candidates.iter().position(|c| c.exists))
            .ok_or("未找到可用 adb（config/env/MuMu/bin 均缺失，PATH 亦未验证通过）")?;
        for c in candidates.iter_mut() {
            c.chosen = false;
        }
        candidates[chosen_idx].chosen = true;
        let adb = PathBuf::from(&candidates[chosen_idx].path);
        let source = candidates[chosen_idx].source.clone();

        // 可用性验证：真的能跑起来
        let out = run_raw(&adb, &["version"], Duration::from_secs(5)).await?;
        if out.timed_out || !out.stdout.contains("Android Debug Bridge") {
            return Err(format!(
                "adb 存在但无法执行：{}（{}）",
                adb.display(),
                out.stderr.trim()
            ));
        }

        Ok(Self {
            adb,
            source,
            candidates,
        })
    }

    pub fn path(&self) -> &Path {
        &self.adb
    }

    /// 带超时执行 adb 子命令（超时强杀子进程，不留悬挂句柄）。
    pub async fn run(&self, args: &[&str], timeout: Duration) -> Result<ProcOutput, String> {
        run_raw(&self.adb, args, timeout).await
    }

    /// adb devices -l 解析。
    pub async fn devices(&self) -> Result<Vec<AdbDevice>, String> {
        let out = self
            .run(&["devices", "-l"], Duration::from_secs(10))
            .await?;
        Ok(parse_devices(&out.stdout))
    }

    /// 单次 connect：15s 硬超时（E-02），成功后必须二次 `adb devices` 实测状态（S-05 假绿灯防护）。
    pub async fn connect(&self, host: &str, port: u16, timeout_s: u64) -> ConnectReport {
        let t0 = std::time::Instant::now();
        let serial = format!("{host}:{port}");
        let mut evidence = vec![format!(
            "$ adb connect {serial}（超时 {timeout_s}s，E-02 硬约束）"
        )];
        let dur = Duration::from_secs(timeout_s.max(1));

        let out = self
            .run(&["connect", &serial], dur)
            .await
            .unwrap_or(ProcOutput {
                code: None,
                stdout: String::new(),
                stderr: "adb 执行失败".into(),
                timed_out: true,
                duration_ms: 0,
            });
        if out.timed_out {
            evidence.push(format!(
                "✖ 超时（{}ms）——非 adbd 端口 connect 会无限挂死，这是 E-02 的典型形态",
                out.duration_ms
            ));
            return ConnectReport {
                ok: false,
                serial,
                state: "timeout".into(),
                attempts: 1,
                evidence,
                elapsed_ms: t0.elapsed().as_millis() as u64,
            };
        }
        let text = format!("{}{}", out.stdout.trim(), out.stderr.trim());
        evidence.push(text.clone());

        // 二次确认（S-05）：独立跑 devices，看 serial 的真实状态
        let devices = self.devices().await.unwrap_or_default();
        let matched = devices.iter().find(|d| d.serial == serial);
        let state = matched
            .map(|d| d.state.clone())
            .unwrap_or_else(|| "missing".into());
        evidence.push(format!("二次确认：adb devices → {serial} = {state}"));

        let ok = state == "device";
        if !ok {
            evidence.push(
                "✖ 假绿灯防护：connect 命令成功 ≠ 实测可通信，以 devices 状态为准（S-05）".into(),
            );
        }
        ConnectReport {
            ok,
            serial,
            state,
            attempts: 1,
            evidence,
            elapsed_ms: t0.elapsed().as_millis() as u64,
        }
    }

    /// offline 自愈曲线（E-03）：disconnect → 2s → connect，×5。
    pub async fn self_heal(&self, host: &str, port: u16, timeout_s: u64) -> ConnectReport {
        let t0 = std::time::Instant::now();
        let serial = format!("{host}:{port}");
        let mut evidence = vec![format!("进入 offline 自愈曲线（E-03）：{serial}")];
        for i in 1..=5u32 {
            evidence.push(format!("— 第 {i}/5 轮 —"));
            let _ = self
                .run(&["disconnect", &serial], Duration::from_secs(5))
                .await;
            evidence.push(format!("  disconnect {serial} 完成"));
            tokio::time::sleep(Duration::from_secs(2)).await;
            let rep = self.connect(host, port, timeout_s).await;
            for e in &rep.evidence {
                evidence.push(format!("  {e}"));
            }
            if rep.ok {
                return ConnectReport {
                    ok: true,
                    serial,
                    state: rep.state,
                    attempts: i,
                    evidence,
                    elapsed_ms: t0.elapsed().as_millis() as u64,
                };
            }
        }
        evidence
            .push("✖ 5 轮自愈未恢复：设备多半未在运行，或该端口不是 adbd（先启动模拟器）".into());
        ConnectReport {
            ok: false,
            serial,
            state: "offline".into(),
            attempts: 5,
            evidence,
            elapsed_ms: t0.elapsed().as_millis() as u64,
        }
    }

    /// 设备 shell（su 通道在调用方组合）。
    pub async fn shell(
        &self,
        serial: &str,
        cmd: &str,
        timeout: Duration,
    ) -> Result<ProcOutput, String> {
        self.run(&["-s", serial, "shell", cmd], timeout).await
    }

    pub async fn push(
        &self,
        serial: &str,
        local: &str,
        remote: &str,
        timeout: Duration,
    ) -> Result<ProcOutput, String> {
        self.run(&["-s", serial, "push", local, remote], timeout)
            .await
    }

    /// 幂等清理：先清残留再建立（文档07：跑第二次就坏 = 不允许）。
    /// pkill -f 自匹配陷阱：模式写成 [f]xxx 使执行 shell 的命令行不命中自身。
    /// A4b：安装链统一走这里（此前 session.rs 内联了同款命令）。
    pub async fn pkill_residue(
        &self,
        serial: &str,
        process_name: &str,
    ) -> Result<ProcOutput, String> {
        let head = process_name
            .chars()
            .next()
            .map(String::from)
            .unwrap_or_default();
        let pattern = format!("[{head}]{}", &process_name[head.len()..]);
        let cmd = format!("su -c 'pkill -f {pattern}; echo done'");
        self.shell(serial, &cmd, Duration::from_secs(10)).await
    }
}

/// 创建不弹控制台窗口的子进程命令（Windows 下 CREATE_NO_WINDOW）。
/// tokio 异步命令的静默版（A4c：裸 spawn 会弹黑窗；reg/where/CIM 均走此处）
pub fn quiet_command(program: impl AsRef<std::ffi::OsStr>) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(program);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    cmd
}

/// 无状态裸执行（detect 阶段还没有 backend 实例时使用）。
pub async fn run_raw(adb: &Path, args: &[&str], timeout: Duration) -> Result<ProcOutput, String> {
    let t0 = std::time::Instant::now();
    let mut cmd = quiet_command(adb);
    cmd.args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    let child = cmd.spawn().map_err(|e| format!("spawn adb 失败: {e}"))?;
    let result = tokio::time::timeout(timeout, child.wait_with_output()).await;
    let duration_ms = t0.elapsed().as_millis() as u64;
    match result {
        Ok(Ok(out)) => Ok(ProcOutput {
            code: out.status.code(),
            stdout: String::from_utf8_lossy(&out.stdout).to_string(),
            stderr: String::from_utf8_lossy(&out.stderr).to_string(),
            timed_out: false,
            duration_ms,
        }),
        Ok(Err(e)) => Err(format!("adb 执行失败: {e}")),
        Err(_) => {
            // 超时：future 被 drop 时 kill_on_drop(true) 已兜底杀掉子进程
            Ok(ProcOutput {
                code: None,
                stdout: String::new(),
                stderr: format!("超时（{}ms）", duration_ms),
                timed_out: true,
                duration_ms,
            })
        }
    }
}

pub fn parse_devices(stdout: &str) -> Vec<AdbDevice> {
    let mut out = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty()
            || line.starts_with("List of devices")
            || line.starts_with('*')
            || line.starts_with("adb")
        {
            continue;
        }
        let mut it = line.split_whitespace();
        let (Some(serial), Some(state)) = (it.next(), it.next()) else {
            continue;
        };
        let model = it
            .find_map(|t| t.strip_prefix("model:"))
            .map(|s| s.to_string());
        out.push(AdbDevice {
            serial: serial.to_string(),
            state: state.to_string(),
            model,
        });
    }
    out
}

/// 扫描常见 MuMu 安装位置（含本机实际布局 D:\System\MuMu\MuMuPlayer\nx_main\adb.exe）。
/// 注册表定位 MuMu 安装目录（A4c：卸载项 InstallLocation，比盘符枚举准且快）。
/// 键名覆盖 MuMu 12 常见安装标识；查不到时调用方回退盘符扫描。
async fn scan_mumu_registry() -> Vec<PathBuf> {
    let keys = [
        r"HKLM\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\MuMuPlayer-12.0",
        r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\MuMuPlayer-12.0",
        r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\MuMuPlayer-12.0",
        r"HKLM\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\MuMu Player",
        r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\MuMu Player",
    ];
    let mut found = Vec::new();
    for key in keys {
        // reg query 走 tokio 进程（P1-5 同款纪律：async 上下文不做同步 spawn 阻塞）
        let output = quiet_command("reg")
            .args(["query", key, "/v", "InstallLocation"])
            .output()
            .await;
        let Ok(out) = output else { continue };
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text.lines() {
            let Some(idx) = line.find("REG_SZ") else {
                continue;
            };
            let dir = line[idx + "REG_SZ".len()..].trim().trim_matches('"');
            if dir.is_empty() {
                continue;
            }
            let base = std::path::PathBuf::from(dir);
            for rel in ["shell/adb.exe", "nx_main/adb.exe", "adb.exe"] {
                let cand = base.join(rel);
                if cand.is_file() {
                    found.push(cand);
                }
            }
        }
    }
    found.sort();
    found.dedup();
    found
}

async fn scan_mumu_adb() -> Vec<PathBuf> {
    // ① 注册表优先（A4c）；② 盘符扫描兜底（绿色版覆盖不到注册表时），盘符扩到 C~G
    let mut found = scan_mumu_registry().await;
    let drives = ["C:", "D:", "E:", "F:", "G:"];
    let roots: Vec<PathBuf> = drives
        .iter()
        .flat_map(|d| {
            [
                PathBuf::from(format!("{d}/Program Files/Netease")),
                PathBuf::from(format!("{d}/Program Files (x86)/Netease")),
                PathBuf::from(format!("{d}/System/MuMu")),
            ]
        })
        .collect();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        for e in entries.flatten() {
            let dir = e.path();
            if !dir.is_dir() {
                continue;
            }
            let name = dir
                .file_name()
                .map(|s| s.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            if !name.contains("mumu") {
                continue;
            }
            for rel in ["shell/adb.exe", "nx_main/adb.exe"] {
                let cand = dir.join(rel);
                if cand.is_file() {
                    found.push(cand);
                }
            }
        }
    }
    found.sort();
    found.dedup();
    found
}

/// 查询正在运行的 adb server 的可执行文件路径（adb.exe 存活进程即 server；
/// 客户端进程转瞬即逝）。结果缓存 30s。失败返回 None。
/// PowerShell CIM 查询可达秒级：走 tokio 进程，不阻塞 worker（P1-5）。
async fn query_running_server_path() -> Option<String> {
    use std::sync::OnceLock;
    use std::time::Instant;
    static CACHE: OnceLock<std::sync::Mutex<Option<(String, Instant)>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(None));
    if let Ok(guard) = cache.lock() {
        if let Some((path, at)) = guard.as_ref() {
            if at.elapsed() < std::time::Duration::from_secs(30) {
                return Some(path.clone());
            }
        }
    }
    let out = {
        let mut cmd = quiet_command("powershell");
        cmd.args([
            "-NoProfile",
            "-Command",
            "Get-CimInstance Win32_Process -Filter \"Name='adb.exe'\" | Sort-Object CreationDate | Select-Object -First 1 -ExpandProperty ExecutablePath",
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
        cmd.output().await.ok()?
    };
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if text.is_empty() {
        return None;
    }
    if let Ok(mut guard) = cache.lock() {
        *guard = Some((text.clone(), Instant::now()));
    }
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "List of devices attached\n\
* daemon not running; starting now at tcp:5037\n\
* daemon started successfully\n\
127.0.0.1:16384 device product:muemu model:MUMU_DEVICE device:muemu transport_id:1\n\
127.0.0.1:16385 offline\n\
emulator-5554 unauthorized\n\n";

    #[test]
    fn parses_states_and_models() {
        let devs = parse_devices(SAMPLE);
        assert_eq!(devs.len(), 3, "标题行/daemon 行/空行必须跳过：{devs:?}");
        assert_eq!(devs[0].serial, "127.0.0.1:16384");
        assert_eq!(devs[0].state, "device");
        assert_eq!(devs[0].model.as_deref(), Some("MUMU_DEVICE"));
        assert_eq!(devs[1].state, "offline");
        assert_eq!(devs[2].state, "unauthorized");
        assert!(devs[2].model.is_none());
    }

    #[test]
    fn empty_and_header_only() {
        assert!(parse_devices("").is_empty());
        assert!(parse_devices("List of devices attached\n").is_empty());
        assert!(parse_devices("* daemon started successfully\n").is_empty());
    }
}
