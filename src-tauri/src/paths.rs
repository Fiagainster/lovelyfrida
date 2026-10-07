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

pub fn bin_dir() -> PathBuf {
    app_root().join("bin")
}

/// 随包资源定位（文档10 P4-2 单安装包）：优先 app_root 直下（开发态/绿色布局）；
/// NSIS 打包对 `../` 资源按 tauri-utils 规则落 `_up_\` 前缀目录，两级都查。
pub fn resource_join(rel: &str) -> PathBuf {
    resource_join_at(app_root(), rel)
}

/// 纯逻辑内核（批次⑭单测锚点）：两级定位语义的单一事实源。
/// `_up_` 兜底曾是批次⑧出货级 bug 的根源（sidecar exe 多一层 dist 导致安装版
/// 永远找不到）——这段逻辑必须有测试钉死，不许再靠真机才能发现回归。
fn resource_join_at(base: &Path, rel: &str) -> PathBuf {
    let direct = base.join(rel);
    if direct.exists() {
        return direct;
    }
    let up = base.join("_up_").join(rel);
    if up.exists() {
        return up;
    }
    direct // 不存在时返回直连路径，让调用方给出明确的缺失报错
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_join_直下优先() {
        let base = std::env::temp_dir().join("lf-test-rj-direct");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("bin/adb/windows-x64")).unwrap();
        std::fs::write(base.join("bin/adb/windows-x64/adb.exe"), b"x").unwrap();
        let got = resource_join_at(&base, "bin/adb/windows-x64/adb.exe");
        assert_eq!(
            got,
            base.join("bin/adb/windows-x64/adb.exe"),
            "存在直下资源时必须返回直下路径"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn resource_join_up_兜底_批次8出货bug回归() {
        let base = std::env::temp_dir().join("lf-test-rj-up");
        let _ = std::fs::remove_dir_all(&base);
        // NSIS 布局：../sidecar/frida_bridge.exe 落在 <install>/_up_/sidecar/
        std::fs::create_dir_all(base.join("_up_/sidecar")).unwrap();
        std::fs::write(base.join("_up_/sidecar/frida_bridge.exe"), b"x").unwrap();
        let got = resource_join_at(&base, "sidecar/frida_bridge.exe");
        assert_eq!(
            got,
            base.join("_up_/sidecar/frida_bridge.exe"),
            "直下缺失时必须命中 _up_ 兜底（批次⑧语义）"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn resource_join_都不存在时返回直连路径供调用方报缺失() {
        let base = std::env::temp_dir().join("lf-test-rj-none");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let got = resource_join_at(&base, "bin/adb/windows-x64/adb.exe");
        assert_eq!(
            got,
            base.join("bin/adb/windows-x64/adb.exe"),
            "缺失时返回直连路径（不返回 _up_ 路径）"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn resource_join_直下优先于_up_() {
        let base = std::env::temp_dir().join("lf-test-rj-both");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("sidecar")).unwrap();
        std::fs::create_dir_all(base.join("_up_/sidecar")).unwrap();
        std::fs::write(base.join("sidecar/frida_bridge.exe"), b"direct").unwrap();
        std::fs::write(base.join("_up_/sidecar/frida_bridge.exe"), b"up").unwrap();
        let got = resource_join_at(&base, "sidecar/frida_bridge.exe");
        assert_eq!(
            got,
            base.join("sidecar/frida_bridge.exe"),
            "两级都存在时直下必须优先"
        );
        let _ = std::fs::remove_dir_all(&base);
    }
}

pub fn bundled_adb_path() -> PathBuf {
    resource_join("bin/adb/windows-x64/adb.exe")
}

pub fn frida_server_matrix_dir() -> PathBuf {
    resource_join("bin/frida-server")
}

/// 打包态 sidecar 可执行（PyInstaller onefile，随 resources 分发）
pub fn sidecar_exe_path() -> PathBuf {
    resource_join("sidecar/frida_bridge.exe")
}

/// 通道B sidecar 脚本（开发态 python 直跑源码）
pub fn sidecar_bridge_path() -> PathBuf {
    app_root().join("sidecar").join("frida_bridge.py")
}

/// sidecar 启动方式（文档10 P4-1：单安装包零外部依赖——exe 优先，开发态回退 python）
#[derive(Debug, Clone)]
pub struct SidecarLaunch {
    pub program: String,
    pub args: Vec<String>,
    /// 供日志/UI 明示启动方式（打包 exe / 开发 python）
    pub label: String,
}

/// 解析 sidecar 启动方式：
/// ① env LOVELYFRIDA_SIDECAR（显式指定 exe）→ ② 安装目录 sidecar\frida_bridge.exe
/// → ③ 开发产物 sidecar\dist\frida_bridge.exe → ④ 系统 python + 源码（需 pip install frida）
pub fn resolve_sidecar_launch(python: &str) -> SidecarLaunch {
    if let Ok(v) = std::env::var("LOVELYFRIDA_SIDECAR") {
        let p = PathBuf::from(v.trim());
        if p.is_file() {
            return SidecarLaunch {
                program: p.display().to_string(),
                args: Vec::new(),
                label: format!("sidecar exe（env 指定）：{}", p.display()),
            };
        }
    }
    let packaged = sidecar_exe_path();
    if packaged.is_file() {
        return SidecarLaunch {
            program: packaged.display().to_string(),
            args: Vec::new(),
            label: format!("sidecar exe（随包）：{}", packaged.display()),
        };
    }
    let dev_exe = app_root()
        .join("sidecar")
        .join("dist")
        .join("frida_bridge.exe");
    if dev_exe.is_file() {
        return SidecarLaunch {
            program: dev_exe.display().to_string(),
            args: Vec::new(),
            label: format!("sidecar exe（dist 产物）：{}", dev_exe.display()),
        };
    }
    SidecarLaunch {
        program: python.to_string(),
        args: vec!["-u".into(), sidecar_bridge_path().display().to_string()],
        label: format!(
            "python 源码模式（开发回退）：{python} -u {}",
            sidecar_bridge_path().display()
        ),
    }
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
        bundled_adb_path()
            .parent()
            .unwrap_or(bin_dir().as_path())
            .to_path_buf(),
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
