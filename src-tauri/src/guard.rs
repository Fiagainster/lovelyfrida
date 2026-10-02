use chrono::Local;
use serde::Serialize;

/// Workspace Guard（文档07 只读第一原则 + M 模块）：
/// - 只读根：在打开文件描述符之前就拒绝写（不是打开后报错）
/// - 强制工作副本：用户发起的处理只接受工作区路径
/// - source\ 只读语义、work\ 可变、破坏性操作前自动快照 snapshots\
///
/// 「纪律无法依赖人在疲劳时保持清醒，只能依赖它在架构上不可能」——路径级拒绝。
#[derive(Debug, Clone, Serialize)]
pub struct GuardVerdict {
    pub allowed: bool,
    pub reason: String,
    pub normalized_path: String,
}

/// 词法规范化：消除 `.`/`..` 与重复分隔符，保证前缀判断（starts_with）不可被
/// `workspace\..\..\evidence` 这类路径构造绕过。canonicalize 不可用时它是最后防线。
fn lexical_clean(p: &std::path::Path) -> std::path::PathBuf {
    use std::path::Component;
    let mut out = std::path::PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                // 越过根的 .. 丢弃：C:\..\x 在 Windows 上语义就是 C:\x
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// 规范化路径：先词法清理，再取最深存在祖先做 canonicalize，
/// 避免「不存在所以无法校验」与「构造相对路径」两类绕过。
pub fn normalize(path: &str) -> std::path::PathBuf {
    normalize_in(&crate::paths::app_root(), path)
}

/// normalize 的纯函数核（A2：可测，不依赖全局 app_root）
pub fn normalize_in(app_root: &std::path::Path, path: &str) -> std::path::PathBuf {
    let p = std::path::Path::new(path);
    let absolute = if p.is_absolute() {
        p.to_path_buf()
    } else {
        app_root.join(p)
    };
    let absolute = lexical_clean(&absolute);
    if let Ok(c) = absolute.canonicalize() {
        return c;
    }
    // 逐级向上找存在的祖先
    let mut probe = absolute.as_path();
    loop {
        match probe.parent() {
            Some(parent) => {
                if let Ok(c) = parent.canonicalize() {
                    return c.join(probe.file_name().unwrap_or_default());
                }
                probe = parent;
            }
            // 无任何存在祖先（极端）：返回词法清理后的路径而不是原始串
            None => return absolute,
        }
    }
}

fn is_under(path: &std::path::Path, root: &std::path::Path) -> bool {
    path.starts_with(root)
}

/// 判定 1+3（检材只读根 / source 段只读语义）：适用于工作区之外的合法写路径
/// （cases\ 台账导出、脚本库、jobs），这些路径不走工作区约束但同样不得触碰检材。
pub fn ensure_not_evidence(path: &str) -> GuardVerdict {
    ensure_not_evidence_in(&crate::config::get(), &crate::paths::app_root(), path)
}

pub fn ensure_not_evidence_in(
    cfg: &crate::config::AppConfig,
    app_root: &std::path::Path,
    path: &str,
) -> GuardVerdict {
    let normalized = normalize_in(app_root, path);

    // 1) 检材只读根：命中即拒绝（机制，不是提醒）
    for r in &cfg.read_only_roots {
        if r.trim().is_empty() {
            continue;
        }
        let root = normalize(r);
        if is_under(&normalized, &root) {
            let reason = format!(
                "检材只读根「{}」：写入被直接拒绝（文档07 只读第一原则）",
                root.display()
            );
            crate::audit::audit("write", &normalized.display().to_string(), "denied", "guard", &reason);
            return GuardVerdict {
                allowed: false,
                reason,
                normalized_path: normalized.display().to_string(),
            };
        }
    }

    // 3) source\ 段只读语义（大小写不敏感，任何层级的 source 目录都是检材副本）
    let has_source_segment = normalized
        .components()
        .any(|c| c.as_os_str().eq_ignore_ascii_case("source"));
    if has_source_segment {
        let reason =
            "source\\ 为检材原始副本（只读语义），请写入 work\\（文档07 只读语义分层）".to_string();
        crate::audit::audit("write", &normalized.display().to_string(), "denied", "guard", &reason);
        return GuardVerdict {
            allowed: false,
            reason,
            normalized_path: normalized.display().to_string(),
        };
    }

    GuardVerdict {
        allowed: true,
        reason: "允许写入（非检材路径）".to_string(),
        normalized_path: normalized.display().to_string(),
    }
}

pub fn check_write(path: &str) -> GuardVerdict {
    check_write_in(&crate::config::get(), &crate::paths::app_root(), path)
}

pub fn check_write_in(
    cfg: &crate::config::AppConfig,
    app_root: &std::path::Path,
    path: &str,
) -> GuardVerdict {
    // 规则 1+3 与非工作区写路径共用同一判定
    let verdict = ensure_not_evidence_in(cfg, app_root, path);
    if !verdict.allowed {
        return verdict;
    }
    let normalized = normalize_in(app_root, path);
    // 工作区根同样过 normalize：Windows canonicalize 产生 \\?\ verbatim 前缀，
    // 与配置字符串直接 starts_with 永不匹配（测试抓出的真 bug，A2）
    let workspace = normalize_in(
        app_root,
        &crate::paths::workspace_root(cfg).display().to_string(),
    );

    // 2) 必须落在工作区（处理后端只接受工作区路径）
    if !is_under(&normalized, &workspace) {
        let reason = format!(
            "只接受工作区路径（{}）；如需处理检材，先复制到工作副本（文档07 强制工作副本）",
            workspace.display()
        );
        crate::audit::audit("write", &normalized.display().to_string(), "denied", "guard", &reason);
        return GuardVerdict {
            allowed: false,
            reason,
            normalized_path: normalized.display().to_string(),
        };
    }

    GuardVerdict {
        allowed: true,
        reason: "允许写入（工作区）".to_string(),
        normalized_path: normalized.display().to_string(),
    }
}

/// 写文件前的高层入口（服务层写路径统一走这里）：
/// 拒绝 = 返回 Err；允许 = 通过。用于 cases\ 下的台账导出 / 脚本库 / jobs / 实验记录。
pub fn guard_write_or_err(path: &std::path::Path) -> Result<(), String> {
    let v = ensure_not_evidence(&path.display().to_string());
    if v.allowed {
        Ok(())
    } else {
        Err(v.reason)
    }
}

/// 破坏性操作（覆盖/删除既有文件）前自动快照（文档06 snapshots\，07 审计）。
pub fn snapshot_file(src: &std::path::Path) -> Result<std::path::PathBuf, String> {
    if !src.is_file() {
        return Err(format!("快照目标不存在或不是文件：{}", src.display()));
    }
    let cfg = crate::config::get();
    let ts = Local::now().format("%Y%m%d-%H%M%S%.3f");
    let dest_dir = crate::paths::workspace_root(&cfg)
        .join("_snapshots")
        .join(ts.to_string());
    std::fs::create_dir_all(&dest_dir).map_err(|e| e.to_string())?;
    let dest = dest_dir.join(
        src.file_name()
            .map(std::ffi::OsStr::to_os_string)
            .ok_or("无文件名")?,
    );
    let bytes = std::fs::read(src).map_err(|e| e.to_string())?;
    // 写前/写后指纹（文档07 审计要求）
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(&bytes);
    let sha = format!("{:x}", h.finalize());
    std::fs::write(&dest, &bytes).map_err(|e| e.to_string())?;
    crate::audit::audit(
        "snapshot",
        &src.display().to_string(),
        "done",
        "guard",
        &format!("{} -> {} sha256={}", src.display(), dest.display(), &sha[..16]),
    );
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use std::path::PathBuf;

    /// 每个测试独立临时根（workspace + evidence 只读根），不依赖全局 config
    fn test_env() -> (AppConfig, PathBuf) {
        let root = std::env::temp_dir().join(format!("lf-guard-test-{}", std::process::id()));
        let workspace = root.join("workspace");
        let evidence = root.join("evidence");
        let _ = std::fs::create_dir_all(&workspace);
        let _ = std::fs::create_dir_all(&evidence);
        let mut cfg = AppConfig::default();
        cfg.workspace_root = workspace.display().to_string();
        cfg.read_only_roots = vec![evidence.display().to_string()];
        (cfg, root)
    }

    #[test]
    fn lexical_clean_kills_traversal() {
        // workspace\..\..\evidence 的构造绕过必须被词法清理戳穿（P1-2）
        let (cfg, root) = test_env();
        // workspace\..\evidence\x 语义上落在检材只读根内，必须被拒绝
        let evil = root.join("workspace").join("..").join("evidence").join("db.sqlite");
        let v = check_write_in(&cfg, &root, &evil.display().to_string());
        assert!(!v.allowed, "穿越到检材只读根必须被拒绝：{}", v.reason);
        // 断言用同一形式：evidence 根也过 normalize（对齐 canonicalize 的 verbatim 前缀）
        let cleaned = normalize_in(&root, &evil.display().to_string());
        let evidence_root = normalize_in(&root, &root.join("evidence").display().to_string());
        assert!(cleaned.starts_with(&evidence_root));
    }

    #[test]
    fn read_only_root_denied() {
        let (cfg, root) = test_env();
        let p = root.join("evidence").join("db.sqlite");
        let v = check_write_in(&cfg, &root, &p.display().to_string());
        assert!(!v.allowed);
        assert!(v.reason.contains("只读根"));
    }

    #[test]
    fn workspace_write_allowed() {
        let (cfg, root) = test_env();
        let p = root.join("workspace").join("work").join("out.txt");
        let v = check_write_in(&cfg, &root, &p.display().to_string());
        assert!(v.allowed, "{}", v.reason);
    }

    #[test]
    fn outside_workspace_denied() {
        let (cfg, root) = test_env();
        let p = root.join("somewhere-else").join("x.txt");
        let v = check_write_in(&cfg, &root, &p.display().to_string());
        assert!(!v.allowed);
        assert!(v.reason.contains("工作区"));
    }

    #[test]
    fn source_segment_denied_even_in_workspace() {
        let (cfg, root) = test_env();
        let p = root.join("workspace").join("source").join("copy.apk");
        let v = check_write_in(&cfg, &root, &p.display().to_string());
        assert!(!v.allowed);
        assert!(v.reason.contains("source"));
    }

    #[test]
    fn source_segment_case_insensitive() {
        let (cfg, root) = test_env();
        let p = root.join("workspace").join("SOURCE").join("x.bin");
        let v = ensure_not_evidence_in(&cfg, &root, &p.display().to_string());
        assert!(!v.allowed, "SOURCE 大小写变体同样拒绝");
    }

    #[test]
    fn evidence_export_allowed_outside_workspace() {
        // cases\ 导出走 ensure_not_evidence：不受工作区约束，但检材根仍拒绝
        let (cfg, root) = test_env();
        let ok = root.join("cases").join("exports").join("x.md");
        let v = ensure_not_evidence_in(&cfg, &root, &ok.display().to_string());
        assert!(v.allowed, "{}", v.reason);
        let bad = root.join("evidence").join("x.md");
        let v2 = ensure_not_evidence_in(&cfg, &root, &bad.display().to_string());
        assert!(!v2.allowed);
    }
}
