use serde::{Deserialize, Serialize};
use std::sync::{OnceLock, RwLock};

/// 应用配置（文档06 config.toml：workspace/cases 不给系统盘、
/// adb_connect_timeout_s=15 为 E-02 硬约束、telemetry 不存在这个开关——故无此字段）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub workspace_root: String,
    pub cases_root: String,
    pub adb_path: String,
    /// auto | a | b | c（D2：通道B优先，auto 先 B）
    pub preferred_channel: String,
    pub frida_port: u16,
    pub adb_connect_timeout_s: u64,
    /// 检材只读根（文档07 只读第一原则）
    pub read_only_roots: Vec<String>,
    pub mask_secrets_in_logs: bool,
    pub retain_raw_log_lines: u32,
    pub ui: UiConfig,
    pub doctor: DoctorConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    pub theme: String,
    pub language: String,
    pub nav_expanded: bool,
}

/// 环境体检配置（可由用户在设置弹窗调整）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DoctorConfig {
    /// 启动时自动跑一次「快速体检」（默认关——快速体检 <2s，但打开应用即弹窗会打扰）
    pub auto_run_on_startup: bool,
    /// 模拟器 adb 端口候选（MuMu 12 默认 16384/16385/7555；可加雷电 5555、夜神 62001 等）
    pub emulator_ports: Vec<u16>,
    /// 用户自定义 adb 候选路径（优先级仅次于 config adb_path）
    pub adb_extra_paths: Vec<String>,
    /// 深度体检的 connect 超时秒数（E-02 硬约束默认 15）
    pub deep_connect_timeout_s: u64,
}

impl Default for DoctorConfig {
    fn default() -> Self {
        Self {
            auto_run_on_startup: false,
            emulator_ports: vec![16384, 16385, 7555],
            adb_extra_paths: Vec::new(),
            deep_connect_timeout_s: 15,
        }
    }
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            theme: "dark".into(),
            language: "zh-CN".into(),
            nav_expanded: true,
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            workspace_root: String::new(),
            cases_root: String::new(),
            adb_path: String::new(),
            preferred_channel: "auto".into(),
            frida_port: 27042,
            adb_connect_timeout_s: 15,
            read_only_roots: Vec::new(),
            mask_secrets_in_logs: true,
            retain_raw_log_lines: 50000,
            ui: UiConfig::default(),
            doctor: DoctorConfig::default(),
        }
    }
}

static CONFIG: OnceLock<RwLock<AppConfig>> = OnceLock::new();

fn cache() -> &'static RwLock<AppConfig> {
    CONFIG.get_or_init(|| RwLock::new(AppConfig::default()))
}

/// 启动时加载；文件缺失则落一份默认配置；损坏则备份后重建（文档07 工程纪律）。
pub fn init() -> Result<(), String> {
    let path = crate::paths::config_path();
    if path.exists() {
        let raw = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        match toml::from_str::<AppConfig>(&raw) {
            Ok(cfg) => {
                *cache().write().map_err(|e| e.to_string())? = cfg;
            }
            Err(e) => {
                let bak = path.with_extension(format!(
                    "toml.bak-{}",
                    chrono::Local::now().format("%Y%m%d-%H%M%S")
                ));
                let _ = std::fs::rename(&path, &bak);
                tracing::warn!(
                    "config.toml 解析失败（{e}），已备份到 {} 并重建默认配置",
                    bak.display()
                );
                let default = AppConfig::default();
                persist(&default)?;
                *cache().write().map_err(|e| e.to_string())? = default;
            }
        }
    } else {
        let default = AppConfig::default();
        persist(&default)?;
        *cache().write().map_err(|e| e.to_string())? = default;
    }
    Ok(())
}

/// 当前配置快照。
pub fn get() -> AppConfig {
    cache().read().map(|c| c.clone()).unwrap_or_default()
}

/// 读-改-写全程持锁（LovelyMem settings.rs 模式），写盘后返回新快照。
pub fn update(f: impl FnOnce(&mut AppConfig)) -> Result<AppConfig, String> {
    let mut guard = cache().write().map_err(|e| e.to_string())?;
    f(&mut guard);
    validate(&guard)?;
    persist(&guard)?;
    Ok(guard.clone())
}

fn validate(cfg: &AppConfig) -> Result<(), String> {
    for (name, p) in [
        ("workspace_root", &cfg.workspace_root),
        ("cases_root", &cfg.cases_root),
    ] {
        let t = p.trim();
        if t.is_empty() {
            continue;
        }
        let lower = t.to_ascii_lowercase();
        if lower.starts_with("c:") || lower.starts_with("c:\\") {
            return Err(format!("{name} 不允许写在系统盘 C:（文档07 工程纪律）"));
        }
    }
    if cfg.adb_connect_timeout_s == 0 || cfg.adb_connect_timeout_s > 60 {
        return Err("adb_connect_timeout_s 必须在 1~60 之间（E-02 硬约束默认 15）".into());
    }
    Ok(())
}

fn persist(cfg: &AppConfig) -> Result<(), String> {
    let raw = toml::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    std::fs::write(crate::paths::config_path(), raw).map_err(|e| e.to_string())
}
