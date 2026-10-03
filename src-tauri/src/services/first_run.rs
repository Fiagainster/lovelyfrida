/// 首启自检（文档02§九 / 07）：目录可写、二进制 sha256、磁盘空间、端口占用。
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct FirstRunItem {
    pub id: String,
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FirstRunReport {
    pub ok: bool,
    pub items: Vec<FirstRunItem>,
}

pub async fn run() -> Result<FirstRunReport, String> {
    let mut items: Vec<FirstRunItem> = Vec::new();

    // FR-01 目录布局
    let layout = crate::paths::ensure_layout();
    items.push(FirstRunItem {
        id: "FR-01".into(),
        name: "目录布局与可写性".into(),
        ok: layout.is_ok(),
        detail: match &layout {
            Ok(()) => format!(
                "bin / workspace / cases / logs 就绪，根目录 {}",
                crate::paths::app_root().display()
            ),
            Err(e) => format!("目录创建失败：{e}"),
        },
    });

    // FR-02 内置二进制 sha256 对账（文档07 供应链）
    items.push(check_binary_hashes().await);

    // FR-03 磁盘空间
    let root = crate::paths::app_root();
    let (disk_ok, disk_detail) = match fs2::available_space(&root)
        .map(|b| b as f64 / 1024.0 / 1024.0 / 1024.0)
    {
        Ok(g) => (g >= 1.0, format!("{root:?} 所在盘剩余 {g:.1} GB（阈值 1 GB）")),
        Err(e) => (false, format!("探测失败：{e}")),
    };
    items.push(FirstRunItem {
        id: "FR-03".into(),
        name: "磁盘空间".into(),
        ok: disk_ok,
        detail: disk_detail,
    });

    // FR-04 cases.db
    let db = crate::store::init();
    items.push(FirstRunItem {
        id: "FR-04".into(),
        name: "cases.db 初始化".into(),
        ok: db.is_ok(),
        detail: match db {
            Ok(()) => format!("SQLite 就绪：{}", crate::paths::cases_db_path().display()),
            Err(ref e) => e.clone(),
        },
    });

    // FR-05 config.toml
    let cfg = crate::config::get();
    items.push(FirstRunItem {
        id: "FR-05".into(),
        name: "config.toml".into(),
        ok: true,
        detail: format!(
            "已加载：preferred_channel={}，adb_connect_timeout_s={}（E-02）",
            cfg.preferred_channel, cfg.adb_connect_timeout_s
        ),
    });

    // FR-06 关键端口占用（本机侧；A4a：读配置端口而非写死 27042）
    let mut port_detail = String::new();
    for p in [cfg.frida_port, cfg.frida_port.saturating_add(1)] {
        let occupied = tokio::net::TcpStream::connect(("127.0.0.1", p)).await.is_ok();
        if occupied {
            port_detail.push_str(&format!("{p} 被占用；"));
            if p == cfg.frida_port {
                port_detail.push_str("S-06：forward 时自动改用备用端口；");
            }
        } else {
            port_detail.push_str(&format!("{p} 空闲；"));
        }
    }
    items.push(FirstRunItem {
        id: "FR-06".into(),
        name: "本机端口占用".into(),
        ok: true, // 占用可自动换端口，不算失败
        detail: port_detail,
    });

    items.push(check_sidecar().await);
    let ok = items.iter().all(|i| i.ok);
    Ok(FirstRunReport { ok, items })
}

/// FR-07 通道B sidecar（文档10 P4-1）：打包 exe 优先；python 回退需 frida 模块可导入。
async fn check_sidecar() -> FirstRunItem {
    let python = std::env::var("LOVELYFRIDA_PYTHON")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "python".into());
    let launch = crate::paths::resolve_sidecar_launch(&python);
    let exe_mode = !launch.program.ends_with(".py");
    if exe_mode {
        let ok = std::path::Path::new(&launch.program).is_file();
        return FirstRunItem {
            id: "FR-07".into(),
            name: "通道B sidecar".into(),
            ok,
            detail: if ok {
                format!("打包 exe 就绪：{}", launch.label)
            } else {
                format!("exe 不存在：{}", launch.program)
            },
        };
    }
    // python 源码模式（开发态）：验证 frida 模块可导入
    let out = crate::backends::adb::run_raw(
        std::path::Path::new(&python),
        &["-c", "import frida; print(frida.__version__)"],
        std::time::Duration::from_secs(30),
    )
    .await;
    let (ok, detail) = match out {
        Ok(o) if o.code == Some(0) && !o.stdout.trim().is_empty() => (
            true,
            format!(
                "python 源码模式（开发回退，正式分发请构建 sidecar exe）：frida {}",
                o.stdout.trim()
            ),
        ),
        Ok(o) => (
            false,
            format!(
                "frida 模块不可用：{} {}",
                o.stdout.trim(),
                o.stderr.trim()
            ),
        ),
        Err(e) => (false, format!("python 执行失败：{e}")),
    };
    FirstRunItem {
        id: "FR-07".into(),
        name: "通道B sidecar".into(),
        ok,
        detail,
    }
}

/// FR-02：bin\adb 三个文件的 sha256 与 binary_manifest.json 对账（资源定位走 resource_join）。
async fn check_binary_hashes() -> FirstRunItem {
    use sha2::{Digest, Sha256};
    let manifest_path = crate::paths::resource_join("bin/binary_manifest.json");
    let bin_base = crate::paths::resource_join("bin");

    if !manifest_path.exists() {
        return FirstRunItem {
            id: "FR-02".into(),
            name: "二进制清单对账".into(),
            ok: false,
            detail: format!("清单缺失：{}", manifest_path.display()),
        };
    }

    let manifest: serde_json::Value = match std::fs::read_to_string(&manifest_path)
        .map_err(|e| e.to_string())
        .and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))
    {
        Ok(m) => m,
        Err(e) => {
            return FirstRunItem {
                id: "FR-02".into(),
                name: "二进制清单对账".into(),
                ok: false,
                detail: format!("清单解析失败：{e}"),
            }
        }
    };

    let files = manifest
        .get("files")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut mismatches: Vec<String> = Vec::new();
    let mut ondemand_skipped: Vec<String> = Vec::new();
    let mut checked = 0usize;
    for f in &files {
        let (Some(rel), Some(expected)) = (
            f.get("path").and_then(|v| v.as_str()),
            f.get("sha256").and_then(|v| v.as_str()),
        ) else {
            continue;
        };
        let p = bin_base.join(rel);
        if !p.exists() {
            // C1 按需模式（瘦身构建）：frida-server 未随包且矩阵目录整体缺失时，
            // 缺文件是预期而非事故——不算对账失败，提示走按需下载
            if rel.starts_with("frida-server/") && !crate::paths::frida_server_matrix_dir().exists() {
                ondemand_skipped.push(rel.to_string());
                continue;
            }
            mismatches.push(format!("{rel}：文件缺失"));
            continue;
        }
        let bytes = std::fs::read(&p).unwrap_or_default();
        let mut h = Sha256::new();
        h.update(&bytes);
        let actual = format!("{:x}", h.finalize());
        if !actual.eq_ignore_ascii_case(expected) {
            mismatches.push(format!("{rel}：sha256 不一致"));
        }
        checked += 1;
    }
    FirstRunItem {
        id: "FR-02".into(),
        name: "二进制清单对账".into(),
        ok: mismatches.is_empty() && checked > 0,
        detail: if mismatches.is_empty() {
            if ondemand_skipped.is_empty() {
                format!("{checked} 个内置二进制 sha256 全部一致")
            } else {
                format!(
                    "{checked} 个随包二进制 sha256 全部一致；{} 个 frida-server 未随包（按需下载模式，体检页可获取）",
                    ondemand_skipped.len()
                )
            }
        } else {
            format!("不一致/缺失：{}", mismatches.join("；"))
        },
    }
}
