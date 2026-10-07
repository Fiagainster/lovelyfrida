//! frida-server 按需下载（docs/10 附二 C1）：manifest 驱动 + sha256 对账（S-01 供应链纪律）。
//!
//! 边界：
//! - 只允许下载 `bin/binary_manifest.json` 里登记过的版本（有 sha256 对账来源才许落地，
//!   裸版本一律拒绝——下载功能不做「任意版本超市」）。
//! - 落点为 `workspace/frida-server/<v>/android-<abi>/`（运行时唯一可写区）；随包 `bin/`
//!   矩阵优先级更高（find_server_binary 先查 bin 再查 workspace）。
//! - 已存在且 sha 一致的文件自动跳过：按钮可反复点，断点语义 = 整文件重下。
use crate::config::AppConfig;
use serde::Serialize;
use sha2::Digest;

#[derive(Debug, Clone, Serialize)]
pub struct FetchEntry {
    pub abi: String,
    /// 落盘相对路径（相对 workspace 根）
    pub path: String,
    pub size: u64,
    pub skipped: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FetchReport {
    pub version: String,
    pub entries: Vec<FetchEntry>,
    pub elapsed_ms: u64,
}

const RELEASE_BASE: &str = "https://github.com/frida/frida/releases/download";

/// manifest 中登记的 (abi → sha256)。版本未登记 → 空表（调用方拒绝下载）。
fn manifest_abis(version: &str) -> Vec<(String, String)> {
    let path = crate::paths::resource_join("bin/binary_manifest.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let Some(files) = v.get("files").and_then(|f| f.as_array()) {
        for e in files {
            let name = e.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let ver = e.get("version").and_then(|n| n.as_str()).unwrap_or("");
            let abi = e.get("abi").and_then(|n| n.as_str());
            let sha = e.get("sha256").and_then(|n| n.as_str());
            if name == "frida-server" && ver == version {
                if let (Some(abi), Some(sha)) = (abi, sha) {
                    out.push((abi.to_string(), sha.to_string()));
                }
            }
        }
    }
    out.sort();
    out
}

fn workspace_target(cfg: &AppConfig, version: &str, abi: &str) -> std::path::PathBuf {
    crate::paths::workspace_root(cfg)
        .join("frida-server")
        .join(version)
        .join(format!("android-{abi}"))
        .join("frida-server")
}

/// 单 ABI：下载（xz）→ 解压 → sha256 对账 → 落盘。阻塞 IO，调用方放 spawn_blocking。
fn fetch_one(
    cfg: &AppConfig,
    version: &str,
    abi: &str,
    want_sha: &str,
) -> Result<FetchEntry, String> {
    let rel = format!("frida-server/{version}/android-{abi}/frida-server");
    // 幂等：已存在且 sha 一致 → 跳过
    let target = workspace_target(cfg, version, abi);
    if target.is_file() {
        let bytes = std::fs::read(&target).map_err(|e| format!("读取已存在文件失败：{e}"))?;
        let got = hex::encode(sha2::Sha256::digest(&bytes));
        if got.eq_ignore_ascii_case(want_sha) {
            return Ok(FetchEntry {
                abi: abi.into(),
                path: rel,
                size: bytes.len() as u64,
                skipped: true,
                error: None,
            });
        }
    }
    let url = format!("{RELEASE_BASE}/{version}/frida-server-{version}-android-{abi}.xz");
    use std::io::Read as _;
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(15))
        .timeout_read(std::time::Duration::from_secs(300))
        .build();
    let resp = agent
        .get(&url)
        .call()
        .map_err(|e| format!("下载失败（{url}）：{e}"))?;
    let mut compressed = Vec::new();
    resp.into_reader()
        .take(200 * 1024 * 1024)
        .read_to_end(&mut compressed)
        .map_err(|e| format!("下载读取失败：{e}"))?;
    let mut plain = Vec::new();
    lzma_rs::xz_decompress(&mut &compressed[..], &mut plain)
        .map_err(|e| format!("xz 解压失败：{e}"))?;
    let got = hex::encode(sha2::Sha256::digest(&plain));
    if !got.eq_ignore_ascii_case(want_sha) {
        return Err(format!(
            "sha256 对账不符（S-01）：期望 {want_sha}，实得 {got}——已丢弃，不落地"
        ));
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败：{e}"))?;
    }
    std::fs::write(&target, &plain).map_err(|e| format!("落盘失败：{e}"))?;
    Ok(FetchEntry {
        abi: abi.into(),
        path: rel,
        size: plain.len() as u64,
        skipped: false,
        error: None,
    })
}

/// 按需下载入口。version 由命令层解析（None → sidecar hello 的客户端版本，S-01 三处一致）；
/// abis=None → manifest 中该版本登记的全部 ABI。
pub async fn fetch(
    cfg: &AppConfig,
    version: String,
    abis: Option<Vec<String>>,
) -> Result<FetchReport, String> {
    let t0 = std::time::Instant::now();
    let version = version.trim().to_string();
    if version.is_empty() {
        return Err("无法确定 frida 客户端版本".into());
    }
    let registered = manifest_abis(&version);
    if registered.is_empty() {
        return Err(format!(
            "版本 {version} 未在 bin/binary_manifest.json 登记（无 sha256 对账来源，S-01 拒绝下载）：\
             如需新版本，请先在清单中登记其 sha256"
        ));
    }
    let targets: Vec<(String, String)> = match abis {
        Some(list) if !list.is_empty() => {
            let want: Vec<String> = list.iter().map(|s| s.trim().to_lowercase()).collect();
            registered
                .into_iter()
                .filter(|(abi, _)| want.iter().any(|w| abi.eq_ignore_ascii_case(w)))
                .collect()
        }
        _ => registered,
    };
    if targets.is_empty() {
        return Err("请求的 ABI 均未在清单登记".into());
    }

    let mut entries: Vec<FetchEntry> = Vec::new();
    for (abi, sha) in targets {
        let cfg2 = cfg.clone();
        let version2 = version.clone();
        let abi2 = abi.clone();
        let sha2s = sha.clone();
        let r = tauri::async_runtime::spawn_blocking(move || {
            fetch_one(&cfg2, &version2, &abi2, &sha2s)
        })
        .await
        .map_err(|e| format!("下载任务失败：{e}"))?;
        match r {
            Ok(e) => entries.push(e),
            Err(e) => entries.push(FetchEntry {
                abi,
                path: format!("frida-server/{version}/android-*/frida-server"),
                size: 0,
                skipped: false,
                error: Some(e),
            }),
        }
    }
    let ok_count = entries.iter().filter(|e| e.error.is_none()).count();
    crate::audit::audit(
        "frida_server_fetch",
        &version,
        if ok_count == entries.len() {
            "done"
        } else {
            "warn"
        },
        "doctor",
        &format!("{ok_count}/{} ABI 成功", entries.len()),
    );
    Ok(FetchReport {
        version,
        entries,
        elapsed_ms: t0.elapsed().as_millis() as u64,
    })
}
