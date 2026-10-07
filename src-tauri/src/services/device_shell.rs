//! 设备侧 shell 命令净化（批次⑩安全封堵）。
//! `adb shell <cmd>` 的命令串由设备端 sh 解析，`su -c '…'` 内层还要再过一次 sh——
//! 两层都是完整 shell 语法。前端传入的 package / 路径 / 文件名一旦含引号、`;`、`$`、
//! 反引号等元字符即可逃逸出预期命令边界，而 su -c 逃逸等于拿到设备 root shell。
//! 纪律：①package 只认 valid_package 白名单（am/monkey 无引号拼接位）；②路径与文件名
//! 一律经 sq() 引用后拼进内层命令，整条内层命令再经 su_c() 引用交给 su——双层各归各的解析器。
//!
//! 正确性自证见文末测试：用 POSIX 引用感知的 token 化器做对抗样本 round-trip，
//! 断言「恶意输入只改变单个 argv 的内容，不改变 argv 的个数与命令边界」。

/// Android 包名/进程名字符集白名单：字母、数字、点、下划线、连字符。
/// 连字符非 shell 元字符、无注入风险，放行是为了兼容 force-stop 原生进程名（frida-server 等）。
/// 安全边界在字符集本身——排除元字符后无论出现在引号内外的哪个拼接位都无法引入 shell 语法；
/// 不做段格式校验（首段数字、单段无点都放行），避免误伤合法用法。
pub fn valid_package(p: &str) -> bool {
    !p.is_empty()
        && p.len() <= 200
        && p.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
}

/// 纯文件名校验：非空、非 "."/".."、不含路径分隔符与 NUL、≤255 字节（ext4 上限）。
/// 实验台用 device_name 拼本机 %TEMP% 文件名（`lf-exp-<name>`），必须先于落盘拦下宿主侧穿越。
pub fn valid_filename(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name.len() <= 255
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains('\0')
}

/// POSIX 单引号引用：整体包 `'…'`，内部 `'` 以 `'\''` 关-转-开惯用法转义
/// （mksh / toybox sh 均支持，Android 设备端 shell 的公共子集）。
pub fn sq(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

/// `su -c` 的外层引用：整条内层命令作为一个 argv 交给 su（su 再交给 `sh -c` 执行）。
/// 内层命令里的动态参数须先用 sq() 引用——两层引用各对应一层 sh 解析，不可省略任一层。
pub fn su_c(inner: &str) -> String {
    format!("su -c {}", sq(inner))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 引用感知的 token 化器：只实现单引号与反斜杠转义（本模块产出的命令只用到这两者）。
    /// 足以验证核心安全性质：恶意输入只改变 argv 的内容，不改变 argv 的个数与命令边界。
    fn tokenize(cmd: &str) -> Vec<String> {
        let mut tokens = Vec::new();
        let mut cur = String::new();
        let mut in_sq = false;
        let mut chars = cmd.chars().peekable();
        while let Some(c) = chars.next() {
            if in_sq {
                if c == '\'' {
                    in_sq = false;
                } else {
                    cur.push(c); // 单引号内一切皆字面量
                }
            } else if c == '\'' {
                in_sq = true;
            } else if c == '\\' {
                if let Some(&n) = chars.peek() {
                    cur.push(n);
                    chars.next();
                }
            } else if c.is_whitespace() {
                if !cur.is_empty() {
                    tokens.push(std::mem::take(&mut cur));
                }
            } else {
                cur.push(c);
            }
        }
        if !cur.is_empty() {
            tokens.push(cur);
        }
        tokens
    }

    #[test]
    fn sq_基础与转义() {
        assert_eq!(sq("/data/data/com.x/files"), "'/data/data/com.x/files'");
        assert_eq!(sq("a'b"), "'a'\\''b'");
        assert_eq!(sq(""), "''");
        assert_eq!(
            sq("$(rm -rf /); `x` \"q\" \\ "),
            "'$(rm -rf /); `x` \"q\" \\ '"
        );
    }

    #[test]
    fn su_c_双层引用后逐层解析还原() {
        // 正常路径：两层 token 化后与手写等价命令完全一致（语义不变性）
        let dir = "/data/data/com.notevault.app/files";
        let inner = format!("mkdir -p {}", sq(dir));
        let outer = su_c(&inner);
        let t1 = tokenize(&outer);
        assert_eq!(
            t1,
            vec!["su", "-c", &inner],
            "外层解析应把整条内层命令还原为单个 argv"
        );
        assert_eq!(
            tokenize(&inner),
            vec!["mkdir", "-p", dir],
            "内层解析应还原出原始路径"
        );
    }

    #[test]
    fn su_c_对抗样本无法逃逸命令边界() {
        let evils = [
            "/data/evil'; rm -rf /; '",
            "/data/evil$(reboot)",
            "/data/evil`reboot`x",
            "/data/evil && cat /data/system/packages.xml",
            "'; reboot; '",
            "a'b'c\"d\"e\\f$HOME",
            "..\\/..\\/escape",
        ];
        for dir in evils {
            let inner = format!("mkdir -p {}", sq(dir));
            let outer = su_c(&inner);
            let t1 = tokenize(&outer);
            assert_eq!(t1.len(), 3, "外层必须仍是 3 个 argv：{outer}");
            assert_eq!(t1[0], "su");
            assert_eq!(t1[1], "-c");
            assert_eq!(t1[2], inner, "内层命令必须完整保留在单个 argv 内");
            let t2 = tokenize(&inner);
            assert_eq!(
                t2.len(),
                3,
                "内层必须仍是 3 个 argv（元字符不得拆分命令）：{inner}"
            );
            assert_eq!(t2[0], "mkdir");
            assert_eq!(t2[1], "-p");
            assert_eq!(t2[2], dir, "第三个 argv 的内容必须与原始输入逐字一致");
        }
    }

    #[test]
    fn valid_package_白名单() {
        let ok = [
            "com.notevault.app",
            "a.b",
            "A1.b_2.C",
            "frida-server",
            "_x",
            "com.0a",
        ];
        for p in ok {
            assert!(valid_package(p), "应放行：{p}");
        }
        let bad = [
            "",
            "com x",
            "com;x",
            "com'x",
            "com\"x",
            "com$x",
            "com`x`",
            "../x",
            "com/x",
            "com\nx",
            "com应用",
            "a;b|c&d",
        ];
        for p in bad {
            assert!(!valid_package(p), "应拒绝：{p}");
        }
        assert!(!valid_package(&"a".repeat(201)), "超长应拒绝");
    }

    #[test]
    fn valid_filename_拦下宿主路径穿越() {
        let ok = ["password.json", "dump-1.bin", "a b.txt", "数据.json"];
        for f in ok {
            assert!(valid_filename(f), "应放行：{f}");
        }
        let bad = [
            "",
            ".",
            "..",
            "a/b",
            "a\\b",
            "..\\x",
            "x\0y",
            "../../etc/passwd",
        ];
        for f in bad {
            assert!(!valid_filename(f), "应拒绝：{f}");
        }
    }
}
