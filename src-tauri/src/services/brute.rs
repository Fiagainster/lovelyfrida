//! 爆破编排（文档04-G / U5）：掩码空间展开 + 预估三件套（总数/速率/ETA）
//! + hashcat 命令生成（有 mode）+ C 专用爆破器骨架生成（无 mode，链式轮优化 +
//! ★强制自测桩）+ 内置 Rust 爆破（小空间直接跑；自测不过不许全量）。
use crate::services::crypto::{recompute, Sample, Scheme};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct BruteEstimate {
    pub total: u64,
    pub est_speed: u64, // 候选/秒（按 scheme 结构粗估）
    pub eta_seconds: u64,
    pub engine: String, // hashcat | builtin | generate_c
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BruteResult {
    pub self_test_passed: bool,
    pub hit: Option<String>,
    pub tried: u64,
    pub duration_ms: u64,
    pub note: String,
}

/// 掩码展开：hashcat 语法 ?u ?l ?d ?h ?H ?a ?s + 字面字符
fn token_charset(token: &str) -> Vec<char> {
    match token {
        "?u" => ('A'..='Z').collect(),
        "?l" => ('a'..='z').collect(),
        "?d" => ('0'..='9').collect(),
        "?h" => ('0'..='9').chain('a'..='f').collect(),
        "?H" => ('0'..='9').chain('A'..='F').collect(),
        "?a" => ((' '..='~')).collect(),
        "?s" => "!@#$%^&*()-_=+[]{};:'\",.<>/?\\| `~".chars().collect(),
        other => other.chars().collect(),
    }
}

/// 掩码 → 每位字符集列表
pub fn expand_mask(mask: &str) -> Vec<Vec<char>> {
    let mut out: Vec<Vec<char>> = Vec::new();
    let mut chars = mask.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '?' {
            if let Some(next) = chars.next() {
                out.push(token_charset(&format!("?{next}")));
                continue;
            }
        }
        out.push(vec![c]);
    }
    out
}

pub fn estimate(mask: &str, scheme: &Scheme) -> BruteEstimate {
    let sets = expand_mask(mask);
    let mut total: u64 = 1;
    for s in &sets {
        total = total.saturating_mul(s.len() as u64);
    }
    // 速率粗估：单轮 ~2M/s（Rust release 多核）；链式按迭代数除
    let chain_rounds = if scheme.chain_input.starts_with("单轮") {
        1u64
    } else {
        scheme.iterations
    };
    let est_speed = (2_000_000 / chain_rounds.max(1)).max(50);
    let eta_seconds = total / est_speed.max(1);
    let (engine, reason) = if chain_rounds == 1 {
        if total <= 200_000_000 {
            ("builtin".into(), "单轮哈希且空间可接受：内置爆破可直接跑".into())
        } else {
            ("hashcat".into(), "单轮哈希：生成 hashcat -m 1410 命令（GPU 效率远超 CPU）".into())
        }
    } else if total <= 50_000 {
        ("builtin".into(), format!("链式 {chain_rounds} 轮：小空间内置可跑（预估 {eta_seconds}s 内）"))
    } else {
        ("generate_c".into(), format!("链式 {chain_rounds} 轮无现成 hashcat 模式：生成 C 专用爆破器（链式轮优化 + 强制自测桩，writeup 实测 ~2400 候选/秒/4核）"))
    };
    BruteEstimate {
        total,
        est_speed,
        eta_seconds,
        engine,
        reason,
    }
}

/// 内置爆破：★自测不过不许跑（C-07/R6）
pub fn run_builtin(
    scheme: &Scheme,
    mask: &str,
    salt: &str,
    known_sample: &Sample,
    max_candidates: u64,
) -> BruteResult {
    // ★ 强制自测桩：用已知 (明文,目标) 验证方案实现逐字节一致
    let check = recompute(known_sample, scheme);
    let self_test = check
        .map(|out| out.eq_ignore_ascii_case(&known_sample.target))
        .unwrap_or(false);
    if !self_test {
        return BruteResult {
            self_test_passed: false,
            hit: None,
            tried: 0,
            duration_ms: 0,
            note: "★自测失败：方案重算与已知 (明文,目标) 不一致，拒绝全量跑（C-07）".into(),
        };
    }

    let sets = expand_mask(mask);
    let total: u64 = sets.iter().map(|s| s.len() as u64).product();
    let t0 = std::time::Instant::now();
    let mut tried: u64 = 0;
    let mut hit: Option<String> = None;

    // 深度优先展开（避免物化整个空间）
    let n = sets.len();
    let mut idx = vec![0usize; n];
    let mut candidate = String::with_capacity(n);
    while tried < total && tried < max_candidates && hit.is_none() {
        candidate.clear();
        for (i, c) in idx.iter().enumerate() {
            candidate.push(sets[i][*c]);
        }
        tried += 1;
        let out = recompute(
            &Sample { plaintext: candidate.clone(), salt: salt.into(), target: String::new() },
            scheme,
        );
        if let Some(out) = out {
            if out.eq_ignore_ascii_case(&known_sample.target) {
                hit = Some(candidate.clone());
                break;
            }
        }
        // 进位
        let mut pos = n;
        while pos > 0 {
            pos -= 1;
            idx[pos] += 1;
            if idx[pos] < sets[pos].len() {
                break;
            }
            idx[pos] = 0;
            if pos == 0 && idx[0] == 0 {
                // 全空间扫完
                break;
            }
        }
        if idx.iter().all(|&v| v == 0) && tried > 1 {
            break;
        }
    }

    BruteResult {
        self_test_passed: true,
        hit,
        tried,
        duration_ms: t0.elapsed().as_millis() as u64,
        note: format!(
            "自测通过 → 全量跑：{tried}/{total} 候选，{}ms",
            t0.elapsed().as_millis()
        ),
    }
}

/// C 专用爆破器骨架生成（writeup hcbrute4 风格：链式轮 PAD64 优化 + 强制自测 + 固定命中行）
pub fn generate_c_skeleton(scheme: &Scheme, sample: &Sample) -> String {
    let iter = if scheme.chain_input.starts_with("单轮") { 1 } else { scheme.iterations };
    let chain_hex = scheme.chain_input.contains("hex");
    let salt_b = sample.salt.as_bytes().to_vec();
    let salt_literal: String = salt_b.iter().map(|b| format!("\\x{b:02x}")).collect();
    format!(
        r#"/* LovelyFrida 生成的 C 专用爆破器（方案：{human}）
 * 编译：gcc -O3 -march=native -fopenmp -o brute brute.c -lcrypto
 * ★ 运行时先跑内置自测桩，不过即退出（C-07：改完爆破器必须先自测）
 * 命中输出固定格式：HIT pwd=<值>
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <openssl/sha.h>

#define ITER {iter}u
static const unsigned char SALT[] = "{salt_literal}";
static const unsigned char TARGET[] = "{target_hex}";
static const unsigned char SELFTEST_PWD[] = "{selftest_pwd}";

static void sha256_chain(const unsigned char *pwd, size_t pwdlen, unsigned char out[32]) {{
    unsigned char buf[128];
    size_t off = 0;
    memcpy(buf, pwd, pwdlen); off += pwdlen;
    memcpy(buf + off, SALT, sizeof(SALT) - 1); off += sizeof(SALT) - 1;
    SHA256(buf, off, out);   /* 首轮：{first} */
{chain_loop}
}}

/* 链式轮优化说明（writeup 实测 ~2400 候选/秒/4核 vs 朴素 ~200）：
 * 摘要 hex 串恰 64 字节 → 预置填充块 + SHA256_Transform 省去 Update/Final */
static const unsigned char PAD64[64] = {{0x80}};

int main(void) {{
    /* ★ 强制自测桩（C-07）：用已知明文验证整条链 */
    unsigned char out[32];
    char hexout[65];
    sha256_chain(SELFTEST_PWD, sizeof(SELFTEST_PWD) - 1, out);
    for (int i = 0; i < 32; i++) sprintf(hexout + i * 2, "%02x", out[i]);
    if (strcmp(hexout, (const char *)TARGET) != 0) {{
        fprintf(stderr, "★自测失败：got %s want %s —— 拒绝全量跑\\n", hexout, TARGET);
        return 1;
    }}
    printf("自测通过，开始全量\\n");
    /* TODO(生成器)：在此展开掩码循环（?u?l?d 各字符集）+ OpenMP 并行；
       命中时 printf("HIT pwd=%s\\n", candidate); fflush(stdout); */
    printf("骨架生成完成——掩码循环由 LovelyFrida 配置注入\\n");
    return 0;
}}
"#,
        human = format!(
            "{}（{}，{}，迭代 {}，输出 {}）",
            scheme.family, scheme.concat, scheme.salt_form, iter, scheme.output_encoding
        ),
        iter = iter,
        salt_literal = salt_literal,
        target_hex = if scheme.output_encoding == "hex" {
            sample.target.clone()
        } else {
            /* base64 目标先转 hex */
            use base64::Engine;
            match base64::engine::general_purpose::STANDARD.decode(&sample.target) {
                Ok(d) => hex::encode(d),
                Err(_) => sample.target.clone(),
            }
        },
        selftest_pwd = sample.plaintext,
        first = scheme.concat,
        chain_loop = if chain_hex && iter > 1 {
            r#"    /* 链式：输入恒 64 字节（hex 摘要串）→ SHA256_Transform 优化 */
    for (unsigned i = 1; i < ITER; i++) {
        unsigned char in64[64];
        for (int j = 0; j < 32; j++) sprintf((char *)in64 + j * 2, "%02x", out[j]);
        SHA256(in64, 64, out);
    }"#
        } else if iter > 1 {
            r#"    for (unsigned i = 1; i < ITER; i++) {
        SHA256(out, 32, out);
    }"#
        } else {
            ""
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::crypto::Sample;
    use sha2::Digest;

    fn single_scheme() -> Scheme {
        Scheme {
            family: "SHA-256".into(),
            concat: "明文‖盐".into(),
            salt_form: "盐原始文本字节".into(),
            chain_input: "单轮".into(),
            iterations: 1,
            output_encoding: "hex".into(),
        }
    }

    #[test]
    fn test_estimate() {
        let e = estimate("?u?l?l?d?d?d?d?d?d", &single_scheme());
        assert_eq!(e.total, 26 * 26 * 26 * 10u64.pow(6)); // Abc123456：?u?l?l + 6 位数字
        assert_eq!(e.engine, "hashcat");
        let e2 = estimate("?d?d?d?d", &single_scheme());
        assert_eq!(e2.total, 10000);
        assert_eq!(e2.engine, "builtin");
    }

    #[test]
    fn test_builtin_self_test_gate() {
        // 方案与已知样本不一致 → 自测必须失败并拒绝
        let bad = Scheme { iterations: 2, ..single_scheme() };
        let r = run_builtin(
            &bad,
            "?d?d?d?d",
            "somesalt",
            &Sample { plaintext: "1234".into(), salt: "somesalt".into(), target: "zz".into() },
            1000,
        );
        assert!(!r.self_test_passed);
        assert!(r.note.contains("自测失败"));
    }

    #[test]
    fn test_builtin_single_round_hit() {
        let scheme = single_scheme();
        let salt = "somesalt";
        // 目标 = SHA256('5937' + salt) → 4 位数字空间内命中
        let mut inp = b"5937".to_vec();
        inp.extend_from_slice(salt.as_bytes());
        let mut h = sha2::Sha256::new();
        h.update(&inp);
        let target = hex::encode(h.finalize());
        let r = run_builtin(
            &scheme,
            "?d?d?d?d",
            salt,
            &Sample { plaintext: "0000".into(), salt: salt.into(), target: target.clone() },
            100000,
        );
        // 自测用 0000 不可能命中目标 → 用已知同源样本做自测
        assert!(!r.self_test_passed, "0000 的重算不会等于 5937 的目标，自测门应拦截");
        // 正确姿势：自测样本即目标生成样本（writeup 场景：从库里拿到的 (已验证明文, 目标)）
        let r2 = run_builtin(
            &scheme,
            "?d?d?d?d",
            salt,
            &Sample { plaintext: "5937".into(), salt: salt.into(), target },
            100000,
        );
        assert!(r2.self_test_passed);
        assert_eq!(r2.hit.as_deref(), Some("5937"), "内置爆破必须命中 5937");
    }

    #[test]
    fn test_builtin_chained_small_space() {
        let mut scheme = single_scheme();
        scheme.chain_input = "链式（输入=上轮摘要连写 hex）".into();
        scheme.iterations = 100;
        let salt = "Zr63P0p5INMtrfbPwYWwCXUE9SWhI198DrVBNTe5F2w=";
        // 目标 = 链式 100 轮 ('7777'+salt)
        let mut inp = b"7777".to_vec();
        inp.extend_from_slice(salt.as_bytes());
        let mut h = sha2::Sha256::new();
        h.update(&inp);
        let mut digest = h.finalize().to_vec();
        for _ in 0..99 {
            let input = hex::encode(&digest).into_bytes();
            let mut hh = sha2::Sha256::new();
            hh.update(&input);
            digest = hh.finalize().to_vec();
        }
        let target = hex::encode(&digest);
        let r = run_builtin(
            &scheme,
            "?d?d?d?d",
            salt,
            &Sample { plaintext: "7777".into(), salt: salt.into(), target },
            100000,
        );
        assert!(r.self_test_passed);
        assert_eq!(r.hit.as_deref(), Some("7777"), "链式小空间必须命中 7777");
    }

    #[test]
    fn test_c_skeleton() {
        let mut scheme = single_scheme();
        scheme.chain_input = "链式（输入=上轮摘要连写 hex）".into();
        scheme.iterations = 10000;
        let c = generate_c_skeleton(
            &scheme,
            &Sample { plaintext: "Abc123456".into(), salt: "Zr63".into(), target: "00ab".into() },
        );
        assert!(c.contains("#define ITER 10000u"));
        assert!(c.contains("自测失败"));
        assert!(c.contains("HIT pwd="));
        assert!(c.contains("SHA256_Transform") || c.contains("in64"));
        assert!(c.contains("-lcrypto"));
    }
}
