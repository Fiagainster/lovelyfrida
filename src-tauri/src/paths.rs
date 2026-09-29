use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static APP_ROOT: OnceLock<PathBuf> = OnceLock::new();

/// 应用根目录解析（文档06/07：全部落项目目录，不写 C 盘用户目录）。
/// 优先级：环境变量 LOVELYFRIDA_ROOT > 从 exe 目录向上找项目标记 > exe 目录。
pub fn app_root() -> &'static Path {
    APP_ROOT.get_or_init(|| {
        if let Ok(v) = std::env::var("LOVELYFRIDA_ROOT") {
            if !v.trim().is_empty() {
                return PathBuf::from(v.trim());
            }
        }
        if let Ok(exe) = std::env::current_exe() {
            let mut dir = exe.parent().map(Path::to_path_buf);
            for _ in 0..6 {
                match dir {
                    Some(d) => {
                        // 项目标记：docs/ 与 package.json 并存
                        if d.join("docs").is_dir() && d.join("package.json").is_file() {
                            return d;
                        }
                        dir = d.parent().map(Path::to_path_buf);
                    }
                    None => break,
                }
            }
        }
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("."))
    })
}

pub fn config_path() -> PathBuf {
    app_root().join("config.toml")
}

pub fn cases_db_path() -> PathBuf {
    app_root().join("cases.db")
}

pub fn logs_dir() -> PathBuf {
    app_root().join("logs")
}

pub fn audit_log_path() -> PathBuf {
    logs_dir().join("audit.log")
}

#[allow(dead_code)] // M5 环形日志清理启用
pub fn app_log_path() -> PathBuf {
    logs_dir().join("app.log")
}

pub fn bin_dir() -> PathBuf {
    app_root().join("bin")
}

pub fn bundled_adb_path() -> PathBuf {
    bin_dir().join("adb").join("windows-x64").join("adb.exe")
}

pub fn frida_server_matrix_dir() -> PathBuf {
    bin_dir().join("frida-server")
}

/// 通道B sidecar 脚本（M5 打包时随 resources 分发）
pub fn sidecar_bridge_path() -> PathBuf {
    app_root().join("sidecar").join("frida_bridge.py")
}

/// 工作区根（文档06：唯一可写区；config 可覆盖，默认 <root>/workspace）
pub fn workspace_root(cfg: &crate::config::AppConfig) -> PathBuf {
    let t = cfg.workspace_root.trim();
    if t.is_empty() {
        app_root().join("workspace")
    } else {
        PathBuf::from(t)
    }
}

/// 归档根（文档06：cases 不可弃，永不随工作区清理）
pub fn cases_root(cfg: &crate::config::AppConfig) -> PathBuf {
    let t = cfg.cases_root.trim();
    if t.is_empty() {
        app_root().join("cases")
    } else {
        PathBuf::from(t)
    }
}

/// 运行时目录布局（文档06）。
pub fn ensure_layout() -> std::io::Result<()> {
    for d in [
        bin_dir(),
        bundled_adb_path().parent().unwrap_or(bin_dir().as_path()).to_path_buf(),
        frida_server_matrix_dir(),
        logs_dir(),
        app_root().join("workspace"),
        app_root().join("cases").join("artifacts"),
        app_root().join("cases").join("traces"),
        app_root().join("cases").join("experiments"),
        app_root().join("cases").join("jobs"),
        app_root().join("cases").join("exports"),
    ] {
        std::fs::create_dir_all(d)?;
    }
    Ok(())
}
