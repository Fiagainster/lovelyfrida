//! 算法还原（文档04-F / U4 / U5 前置）：给定 (明文, 盐, 目标值) 把算法定死。
//! 穷举维度：哈希族 × 拼接顺序 × 盐形态 × 链式输入形态 × 迭代次数（增量扫描）
//! × 输出编码。防假命中（文档04-F）：**两条不同的 (明文,目标) 都要重算通过**。
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Family {
    Sha256,
    Sha1,
    Md5,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Concat {
    PwSalt,
    SaltPw,
    PwOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SaltForm {
    /// 盐按原始文本字节（如 base64 字符串直接连写）
    RawText,
    /// 盐 base64 解码后的原始字节
    B64Decoded,
    /// 盐 base64 解码后再转连写 hex 文本
    HexText,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ChainInput {
    /// 单轮（无链）
    Single,
    /// 链式：下一轮输入 = 上一轮摘要的连写 hex 字符串
    HexString,
    /// 链式：下一轮输入 = 上一轮摘要原始字节
    RawBytes,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scheme {
    pub family: String,
    pub concat: String,
    pub salt_form: String,
    pub chain_input: String,
    pub iterations: u64,
    pub output_encoding: String, // hex | base64
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sample {
    pub plaintext: String,
    pub salt: String,
    pub target: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconstructResult {
    pub scheme: Option<Scheme>,
    /// 全部命中的候选（多方案同真时全列，防过度泛化 R5）
    pub candidates: Vec<Scheme>,
    pub self_test_passed: bool,
    pub human_desc: String,
    pub python_skeleton: String,
    pub hashcat_mode: Option<String>,
    pub hashcat_cmd: Option<String>,
    pub error: Option<String>,
}

fn hash_bytes(family: Family, data: &[u8]) -> Vec<u8> {
    match family {
        Family::Sha256 => {
            let mut h = Sha256::new();
            h.update(data);
            h.finalize().to_vec()
        }
        Family::Sha1 => {
            let mut h = sha1::Sha1::new();
            h.update(data);
            h.finalize().to_vec()
        }
        Family::Md5 => {
            let mut h = md5::Md5::new();
            h.update(data);
            h.finalize().to_vec()
        }
    }
}

fn b64_encode(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(data)
}

fn salt_bytes(salt: &str, form: SaltForm) -> Vec<u8> {
    match form {
        SaltForm::RawText => salt.as_bytes().to_vec(),
        SaltForm::B64Decoded => {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD
                .decode(salt.trim())
                .unwrap_or_default()
        }
        SaltForm::HexText => {
            use base64::Engine;
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(salt.trim())
                .unwrap_or_default();
            hex::encode(decoded).into_bytes()
        }
    }
}

/// 按 Scheme 的字符串字段解析枚举（recompute / C 骨架生成共用；未知值 None）
pub fn parse_concat(s: &str) -> Option<Concat> {
    match s {
        "明文‖盐" => Some(Concat::PwSalt),
        "盐‖明文" => Some(Concat::SaltPw),
        "仅明文" => Some(Concat::PwOnly),
        _ => None,
    }
}

pub fn parse_salt_form(s: &str) -> Option<SaltForm> {
    match s {
        "盐原始文本字节" => Some(SaltForm::RawText),
        "盐 base64 解码字节" => Some(SaltForm::B64Decoded),
        "盐解码后连写 hex 文本" => Some(SaltForm::HexText),
        _ => None,
    }
}

/// 方案语义下的盐字节：C 爆破器骨架与 Rust recompute 必须同源，
/// 否则 B64Decoded/HexText 方案的骨架自测必挂
pub fn scheme_salt_bytes(scheme: &Scheme, salt: &str) -> Vec<u8> {
    salt_bytes(salt, parse_salt_form(&scheme.salt_form).unwrap_or(SaltForm::RawText))
}

/// 首轮输入（按拼接顺序）
fn first_input(plaintext: &str, salt_b: &[u8], concat: Concat) -> Vec<u8> {
    match concat {
        Concat::PwSalt => {
            let mut v = plaintext.as_bytes().to_vec();
            v.extend_from_slice(salt_b);
            v
        }
        Concat::SaltPw => {
            let mut v = salt_b.to_vec();
            v.extend_from_slice(plaintext.as_bytes());
            v
        }
        Concat::PwOnly => plaintext.as_bytes().to_vec(),
    }
}

const MAX_ITER: u64 = 1_000_000;

/// 对一个组合做增量链式扫描，返回命中的 (chain_input, iterations, out_enc) 列表
fn scan_combo(
    family: Family,
    plaintext: &str,
    salt_b: &[u8],
    concat: Concat,
    target: &str,
    max_iter: u64,
) -> Vec<(ChainInput, u64, &'static str)> {
    let mut hits = Vec::new();
    let h0 = hash_bytes(family, &first_input(plaintext, salt_b, concat));

    // 单轮检查（两种输出编码）
    if hex::encode(&h0).eq_ignore_ascii_case(target) {
        hits.push((ChainInput::Single, 1, "hex"));
    }
    if b64_encode(&h0) == target {
        hits.push((ChainInput::Single, 1, "base64"));
    }
    if !hits.is_empty() {
        return hits;
    }

    // 链式扫描（增量：h_{i+1} = f(h_i)），每轮检查两种输出编码
    for chain in [ChainInput::HexString, ChainInput::RawBytes] {
        let mut h = h0.clone();
        for i in 2..=max_iter.min(MAX_ITER) {
            let input: Vec<u8> = match chain {
                ChainInput::HexString => hex::encode(&h).into_bytes(),
                ChainInput::RawBytes => h.clone(),
                ChainInput::Single => unreachable!(),
            };
            h = hash_bytes(family, &input);
            if hex::encode(&h).eq_ignore_ascii_case(target) {
                hits.push((chain, i, "hex"));
                return hits;
            }
            if b64_encode(&h) == target {
                hits.push((chain, i, "base64"));
                return hits;
            }
        }
    }
    hits
}

fn family_name(f: Family) -> &'static str {
    match f {
        Family::Sha256 => "SHA-256",
        Family::Sha1 => "SHA-1",
        Family::Md5 => "MD5",
    }
}
fn concat_name(c: Concat) -> &'static str {
    match c {
        Concat::PwSalt => "明文‖盐",
        Concat::SaltPw => "盐‖明文",
        Concat::PwOnly => "仅明文",
    }
}
fn salt_form_name(s: SaltForm) -> &'static str {
    match s {
        SaltForm::RawText => "盐原始文本字节",
        SaltForm::B64Decoded => "盐 base64 解码字节",
        SaltForm::HexText => "盐解码后连写 hex 文本",
    }
}
fn chain_name(c: ChainInput) -> &'static str {
    match c {
        ChainInput::Single => "单轮",
        ChainInput::HexString => "链式（输入=上轮摘要连写 hex）",
        ChainInput::RawBytes => "链式（输入=上轮摘要原始字节）",
    }
}

/// 用方案重算（校验用）
pub fn recompute(sample: &Sample, scheme: &Scheme) -> Option<String> {
    let fam = match scheme.family.as_str() {
        "SHA-256" => Family::Sha256,
        "SHA-1" => Family::Sha1,
        "MD5" => Family::Md5,
        _ => return None,
    };
    let concat = parse_concat(&scheme.concat)?;
    let salt_form = parse_salt_form(&scheme.salt_form)?;
    let salt_b = salt_bytes(&sample.salt, salt_form);
    let mut h = hash_bytes(fam, &first_input(&sample.plaintext, &salt_b, concat));
    if scheme.chain_input.starts_with("单轮") {
        // i=1
    } else {
        let hex_chain = scheme.chain_input.contains("hex");
        for _ in 2..=scheme.iterations {
            let input: Vec<u8> = if hex_chain {
                hex::encode(&h).into_bytes()
            } else {
                h.clone()
            };
            h = hash_bytes(fam, &input);
        }
    }
    match scheme.output_encoding.as_str() {
        "hex" => Some(hex::encode(&h)),
        _ => Some(b64_encode(&h)),
    }
}

/// 还原入口：两组样本都必须命中同一方案（防假命中，文档04-F）
pub fn reconstruct(samples: &[Sample]) -> ReconstructResult {
    if samples.is_empty() {
        return empty_result("至少需要一组 (明文, 盐, 目标值)");
    }
    let s0 = &samples[0];
    let mut all_candidates: Vec<Scheme> = Vec::new();

    for family in [Family::Sha256, Family::Sha1, Family::Md5] {
        for concat in [Concat::PwSalt, Concat::SaltPw, Concat::PwOnly] {
            for salt_form in [SaltForm::RawText, SaltForm::B64Decoded, SaltForm::HexText] {
                let salt_b = salt_bytes(&s0.salt, salt_form);
                for (chain, iters, enc) in scan_combo(family, &s0.plaintext, &salt_b, concat, &s0.target, MAX_ITER) {
                    all_candidates.push(Scheme {
                        family: family_name(family).into(),
                        concat: concat_name(concat).into(),
                        salt_form: salt_form_name(salt_form).into(),
                        chain_input: chain_name(chain).into(),
                        iterations: iters,
                        output_encoding: enc.into(),
                    });
                }
            }
        }
    }

    if all_candidates.is_empty() {
        return empty_result("穷举完成未命中：尝试扩展哈希族/迭代上限，或目标值不是纯哈希输出（可能还有额外编码层，C-08）");
    }

    // 双样本防假命中：取第一组命中的方案，逐个用其余样本复核
    let verified: Vec<Scheme> = all_candidates
        .iter()
        .filter(|sch| {
            samples.iter().skip(1).all(|s| {
                recompute(s, sch)
                    .map(|out| out.eq_ignore_ascii_case(&s.target))
                    .unwrap_or(false)
            })
        })
        .cloned()
        .collect();

    if samples.len() > 1 && verified.is_empty() {
        return empty_result("第一组命中但第二组样本未通过同一方案（假命中已排除）——请核对两组样本是否同源同算法");
    }

    let scheme = verified[0].clone();
    let self_test = {
        // 自证：用方案重算两组样本逐字节一致
        samples
            .iter()
            .all(|s| recompute(s, &scheme).map(|out| out.eq_ignore_ascii_case(&s.target)).unwrap_or(false))
    };

    let human = format!(
        "存储值 = {}( {} )，{}，{}；迭代 {} 次；输出 {} 编码",
        scheme.family, scheme.concat, scheme.salt_form, scheme.chain_input, scheme.iterations, scheme.output_encoding
    );

    // Python 验证骨架（文档04-F：规格书三件套之二）
    let py = gen_python_skeleton(&scheme);

    // hashcat 适配判断（文档04-G：有 mode 出命令）
    let (hc_mode, hc_cmd) = hashcat_for(&scheme, &samples[0]);

    ReconstructResult {
        scheme: Some(scheme),
        candidates: verified,
        self_test_passed: self_test,
        human_desc: human,
        python_skeleton: py,
        hashcat_mode: hc_mode,
        hashcat_cmd: hc_cmd,
        error: None,
    }
}

fn empty_result(err: &str) -> ReconstructResult {
    ReconstructResult {
        scheme: None,
        candidates: vec![],
        self_test_passed: false,
        human_desc: String::new(),
        python_skeleton: String::new(),
        hashcat_mode: None,
        hashcat_cmd: None,
        error: Some(err.into()),
    }
}

fn gen_python_skeleton(s: &Scheme) -> String {
    let chain_py = if s.chain_input.starts_with("单轮") {
        "h = hashlib.sha256(payload).digest()  # ← 按实际 family 替换".to_string()
    } else {
        format!(
            "h = hashlib.sha256(payload).digest()\nfor _ in range({} - 1):\n    h = hashlib.sha256({}).digest()",
            s.iterations,
            if s.chain_input.contains("hex") {
                "binascii.hexlify(h)"  // hex 文本输入
            } else {
                "h" // 原始字节输入
            }
        )
    };
    let out_py = if s.output_encoding == "hex" {
        "binascii.hexlify(h).decode()"
    } else {
        "base64.b64encode(h).decode()"
    };
    format!(
        "#!python3\n# LovelyFrida 算法规格书 · 验证骨架\nimport hashlib, binascii, base64\n\nplaintext = b'...'\nsalt = b'...'\n# 组装 payload（{concat}）：\npayload = plaintext + salt\n\n{chain}\n\nresult = {out}\nprint(result)\n# 逐字节自证：result == 目标值 → True\n",
        concat = s.concat,
        chain = chain_py,
        out = out_py
    )
}

/// hashcat 适配：单轮 SHA-256(pw‖salt) → -m 1410；链式/其他 → 无现成模式
fn hashcat_for(s: &Scheme, sample: &Sample) -> (Option<String>, Option<String>) {
    if s.family == "SHA-256" && s.concat == "明文‖盐" && s.chain_input.starts_with("单轮") {
        // 目标可能是 base64 → 转 hex；hashcat -m 1410 格式 hash:salt（均 hex）
        let hash_hex = if s.output_encoding == "base64" {
            use base64::Engine;
            match base64::engine::general_purpose::STANDARD.decode(&sample.target) {
                Ok(d) => hex::encode(d),
                Err(_) => sample.target.clone(),
            }
        } else {
            sample.target.clone()
        };
        let salt_hex = hex::encode(salt_bytes(
            &sample.salt,
            match s.salt_form.as_str() {
                "盐原始文本字节" => SaltForm::RawText,
                "盐 base64 解码字节" => SaltForm::B64Decoded,
                _ => SaltForm::HexText,
            },
        ));
        let cmd = format!(
            "hashcat -m 1410 -a 3 {hash_hex}:{salt_hex} '<掩码>'  # 示例掩码：?u?l?l?d?d?d?d?d?d"
        );
        return (Some("1410 (sha256($pass.$salt))".into()), Some(cmd));
    }
    (None, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// APK-1（writeup 实测数据）：Base64(SHA-256('Wei123123' + 'etmLYLvSIJn2mzgC'))
    #[test]
    fn test_apk1_single_round_base64() {
        let samples = vec![Sample {
            plaintext: "Wei123123".into(),
            salt: "etmLYLvSIJn2mzgC".into(),
            target: "1Q0Tvj9/GLaF+cTMH+2c8z8ieGV3n4L+jySUZLWBcUc=".into(),
        }];
        let r = reconstruct(&samples);
        assert!(r.error.is_none(), "error: {:?}", r.error);
        let s = r.scheme.unwrap();
        assert_eq!(s.family, "SHA-256");
        assert_eq!(s.concat, "明文‖盐");
        assert_eq!(s.output_encoding, "base64");
        assert_eq!(s.iterations, 1);
        assert!(r.self_test_passed);
        assert!(r.hashcat_mode.is_some(), "单轮 SHA-256(pw‖salt) 应有 hashcat 1410");
        println!("APK-1 方案: {}", r.human_desc);
    }

    /// APK-18 风格（合成目标）：h1=SHA256(pw‖salt_b64text)；h_{i+1}=SHA256(hex(h_i))；存储=hex(h_10000)
    #[test]
    fn test_apk18_chained_hex_10000() {
        let pw = "Abc123456";
        let salt = "Zr63P0p5INMtrfbPwYWwCXUE9SWhI198DrVBNTe5F2w=";
        // 按文档04-F 的坑：链式计数从 h2 开始到 h_10000 = 9999 次再哈希
        let mut h = {
            let mut inp = pw.as_bytes().to_vec();
            inp.extend_from_slice(salt.as_bytes());
            let mut hh = Sha256::new();
            hh.update(&inp);
            hh.finalize().to_vec()
        };
        for _ in 0..9999 {
            let input = hex::encode(&h).into_bytes();
            let mut hh = Sha256::new();
            hh.update(&input);
            h = hh.finalize().to_vec();
        }
        let target = hex::encode(&h);

        let samples = vec![
            Sample { plaintext: pw.into(), salt: salt.into(), target: target.clone() },
            // 双样本防假命中：另一组同方案数据
            Sample { plaintext: "Test9876".into(), salt: salt.into(), target: {
                let mut inp = b"Test9876".to_vec();
                inp.extend_from_slice(salt.as_bytes());
                let mut hh = Sha256::new();
                hh.update(&inp);
                let mut hh2 = hh.finalize().to_vec();
                for _ in 0..9999 {
                    let input = hex::encode(&hh2).into_bytes();
                    let mut h3 = Sha256::new();
                    h3.update(&input);
                    hh2 = h3.finalize().to_vec();
                }
                hex::encode(&hh2)
            } },
        ];
        let r = reconstruct(&samples);
        assert!(r.error.is_none(), "error: {:?}", r.error);
        let s = r.scheme.unwrap();
        assert_eq!(s.family, "SHA-256");
        assert_eq!(s.iterations, 10000, "迭代数必须精确到 10000（差 1 即错，writeup 的坑）");
        assert!(s.chain_input.contains("hex"), "链式输入应为连写 hex");
        assert_eq!(s.output_encoding, "hex");
        assert!(r.self_test_passed);
        assert!(r.hashcat_mode.is_none(), "链式无现成 hashcat 模式");
        assert_eq!(r.candidates.len(), 1, "双样本应收敛到唯一方案");
        println!("APK-18 方案: {}", r.human_desc);
    }
}
