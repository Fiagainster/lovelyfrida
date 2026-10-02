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

/// C 专用爆破器骨架生成（writeup hcbrute4 风格：链式轮优化 + 强制自测 + 掩码循环 + OpenMP）。
/// P2-2（文档10）：此前 main() 只有自测桩，掩码循环是 TODO 空壳——现在完整生成。
pub fn generate_c_skeleton(scheme: &Scheme, sample: &Sample, mask: &str) -> String {
    let iter = if scheme.chain_input.starts_with("单轮") { 1 } else { scheme.iterations };
    let chain_hex = scheme.chain_input.contains("hex");
    // 家族 → 摘要长度 / OpenSSL 一次性函数（SHA-256 之外同样成立；自测门兜底正确性）
    let (digest_len, hash_fn, hash_include, family_note) = match scheme.family.as_str() {
        "SHA-1" => (20usize, "SHA1", "#include <openssl/sha.h>", "SHA-1（20 字节摘要）"),
        "MD5" => (16, "MD5", "#include <openssl/md5.h>", "MD5（16 字节摘要）"),
        _ => (32, "SHA256", "#include <openssl/sha.h>", "SHA-256（32 字节摘要）"),
    };
    let hex_len = digest_len * 2;

    // 目标统一转小写 hex（base64 目标先解码；自测/比对都在 hex 域进行）
    let target_hex = if scheme.output_encoding == "hex" {
        sample.target.to_lowercase()
    } else {
        use base64::Engine;
        match base64::engine::general_purpose::STANDARD.decode(&sample.target) {
            Ok(d) => hex::encode(d),
            Err(_) => sample.target.clone(),
        }
    };

    // 掩码展开 → 每位字符集（与内置爆破同一套 token 语义）
    let sets = expand_mask(mask);
    let positions = sets.len();
    let total: u64 = sets.iter().map(|s| s.len() as u64).product();
    // 字符集用数值字节数组，规避 C 字符串 \x 转义的「贪心吞位」与引号转义问题
    let set_decls: String = sets
        .iter()
        .enumerate()
        .map(|(i, cs)| {
            let bytes: Vec<String> = cs.iter().map(|c| format!("{}", *c as u32)).collect();
            format!("static const char SET_{i}[] = {{{}, 0}};", bytes.join(","))
        })
        .collect::<Vec<_>>()
        .join("\n");
    let set_refs: String = (0..positions).map(|i| format!("SET_{i}")).collect::<Vec<_>>().join(", ");
    let set_lens: String = sets.iter().map(|s| format!("{}", s.len())).collect::<Vec<_>>().join(", ");
    let salt_b = sample.salt.as_bytes().to_vec();
    let salt_bytes: Vec<String> = salt_b.iter().map(|b| format!("0x{b:02x}")).collect();

    // 链式轮：hex 链输入是 2×摘要长的 hex 串；原始链输入直接用摘要（经 tmp 防止 in==out 别名）
    let chain_loop = if iter > 1 && chain_hex {
        format!(
            r#"    /* 链式（输入=上轮摘要连写 hex，{hl} 字节） */
    for (unsigned i = 1; i < ITER; i++) {{
        unsigned char in{hl}[{hl}];
        for (int j = 0; j < DIGEST_LEN; j++) {{
            in{hl}[j * 2]     = (unsigned char)((out[j] >> 4) & 0xf) < 10
                                  ? (unsigned char)('0' + ((out[j] >> 4) & 0xf))
                                  : (unsigned char)('a' + ((out[j] >> 4) & 0xf) - 10);
            in{hl}[j * 2 + 1] = (unsigned char)((out[j] & 0xf)) < 10
                                  ? (unsigned char)('0' + (out[j] & 0xf))
                                  : (unsigned char)('a' + (out[j] & 0xf) - 10);
        }}
        {fn}(in{hl}, {hl}, out);
    }}"#,
            hl = hex_len,
            fn = hash_fn,
        )
    } else if iter > 1 {
        format!(
            r#"    /* 链式（输入=上轮原始摘要，经 tmp 防 in==out 别名） */
    for (unsigned i = 1; i < ITER; i++) {{
        unsigned char tmp[DIGEST_LEN];
        memcpy(tmp, out, DIGEST_LEN);
        {fn}(tmp, DIGEST_LEN, out);
    }}"#,
            fn = hash_fn,
        )
    } else {
        String::new()
    };

    format!(
        r#"/* LovelyFrida 生成的 C 专用爆破器（方案：{human}）
 * 编译：gcc -O3 -march=native -fopenmp -o brute brute.c -lcrypto
 * ★ 运行时先跑内置自测桩，不过即退出（C-07：改完爆破器必须先自测）
 * 命中输出固定格式：HIT pwd=<值>
 * 家族说明：{family_note}
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
{hash_include}
#include <omp.h>

#define ITER {iter}u
#define DIGEST_LEN {digest_len}
#define POSITIONS {positions}
#define TOTAL_SPACE {total}ULL
static const unsigned char SALT[] = {{{salt_bytes}}};   /* 盐原始字节（{salt_form}） */
static const char TARGET[] = "{target_hex}";
static const char SELFTEST_PWD[] = "{selftest_pwd}";

{set_decls}
static const char *SETS[POSITIONS] = {{{set_refs}}};
static const int SET_LEN[POSITIONS] = {{{set_lens}}};

static void sha_chain(const unsigned char *pwd, size_t pwdlen, unsigned char out[DIGEST_LEN]) {{
    unsigned char buf[512];
    size_t off = 0;
    if (pwdlen + sizeof(SALT) > sizeof(buf)) return; /* 候选+盐超缓冲：直接放弃该候选 */
    memcpy(buf, pwd, pwdlen); off += pwdlen;
    memcpy(buf + off, SALT, sizeof(SALT)); off += sizeof(SALT);
    {hash_fn}(buf, off, out);   /* 首轮：{first} */
{chain_loop}
}}

static volatile int found = 0;

static void hexify(const unsigned char *digest, char *hexout) {{
    for (int i = 0; i < DIGEST_LEN; i++)
        sprintf(hexout + i * 2, "%02x", digest[i]);
}}

int main(void) {{
    /* ★ 强制自测桩（C-07）：用已知明文验证整条链 */
    unsigned char out[DIGEST_LEN];
    char hexout[DIGEST_LEN * 2 + 1];
    sha_chain((const unsigned char *)SELFTEST_PWD, sizeof(SELFTEST_PWD) - 1, out);
    hexify(out, hexout);
    if (strcmp(hexout, TARGET) != 0) {{
        fprintf(stderr, "★自测失败：got %s want %s —— 拒绝全量跑\n", hexout, TARGET);
        return 1;
    }}
    printf("自测通过，开始全量：%llu 个候选（%d 位掩码）\n",
           (unsigned long long)TOTAL_SPACE, POSITIONS);

    /* 掩码循环：首位的字符集下标做 OpenMP 并行分片，其余位为串行里程表 */
    #pragma omp parallel for schedule(dynamic)
    for (int first = 0; first < SET_LEN[0]; first++) {{
        if (found) continue;
        int idx[POSITIONS];
        char cand[POSITIONS + 1];
        unsigned char out[DIGEST_LEN];
        char hexout[DIGEST_LEN * 2 + 1];
        idx[0] = first;
        for (int i = 1; i < POSITIONS; i++) idx[i] = 0;
        cand[POSITIONS] = '\0';
        for (;;) {{
            for (int i = 0; i < POSITIONS; i++) cand[i] = SETS[i][idx[i]];
            sha_chain((const unsigned char *)cand, POSITIONS, out);
            hexify(out, hexout);
            if (strcmp(hexout, TARGET) == 0) {{
                #pragma omp critical
                {{
                    if (!found) {{
                        printf("HIT pwd=%s\n", cand);
                        fflush(stdout);
                        found = 1;
                    }}
                }}
            }}
            /* 里程表：推进第 1..POSITIONS-1 位；全部回卷则本分片结束 */
            int pos = POSITIONS - 1;
            while (pos >= 1) {{
                if (++idx[pos] < SET_LEN[pos]) break;
                idx[pos] = 0;
                pos--;
            }}
            if (pos < 1) break;
        }}
    }}
    if (!found) printf("全空间扫完，未命中\n");
    return 0;
}}
"#,
        human = format!(
            "{}（{}，{}，迭代 {}，输出 {}）",
            scheme.family, scheme.concat, scheme.salt_form, iter, scheme.output_encoding
        ),
        family_note = family_note,
        hash_include = hash_include,
        iter = iter,
        digest_len = digest_len,
        positions = positions,
        total = total,
        salt_bytes = salt_bytes.join(","),
        salt_form = scheme.salt_form,
        target_hex = target_hex,
        selftest_pwd = sample.plaintext,
        set_decls = set_decls,
        set_refs = set_refs,
        set_lens = set_lens,
        hash_fn = hash_fn,
        first = scheme.concat,
        chain_loop = chain_loop,
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
            "?u?l?l?d?d?d?d?d?d",
        );
        assert!(c.contains("#define ITER 10000u"));
        assert!(c.contains("自测失败"));
        assert!(c.contains("HIT pwd="));
        assert!(c.contains("omp parallel for"), "掩码循环必须带 OpenMP 并行");
        assert!(c.contains("static const char SET_0[]"), "掩码字符集必须展开为 C 数组");
        assert!(c.contains("里程表"), "首位之外必须是串行里程表推进");
        assert!(!c.contains("TODO"), "骨架不允许残留 TODO 空壳");
        assert!(c.contains("-lcrypto"));
        // 盐必须用数值字节数组（\x 十六进制转义会贪心吞后续字节）
        assert!(c.contains("static const unsigned char SALT[] = {0x"));
    }

    #[test]
    fn test_c_skeleton_single_position_mask() {
        // 1 位掩码：每个分片恰一个候选，不能死循环也不能漏
        let scheme = single_scheme();
        let salt = "s";
        let mut inp = b"7".to_vec();
        inp.extend_from_slice(salt.as_bytes());
        let mut h = sha2::Sha256::new();
        h.update(&inp);
        let target = hex::encode(h.finalize());
        let c = generate_c_skeleton(
            &scheme,
            &Sample { plaintext: "7".into(), salt: salt.into(), target },
            "?d",
        );
        assert!(c.contains("#define POSITIONS 1"));
        assert!(c.contains("for (int first = 0; first < SET_LEN[0]; first++)"));
    }
}
