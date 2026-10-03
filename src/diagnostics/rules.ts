import type {
  DoctorReport,
  InjectionReport,
  ProbeStat,
  ServerStatusReport,
  SessionSnapshot,
} from "@/api";

/**
 * 诊断引擎（文档05 · P2-1 数据驱动化，文档10）：
 * 48 条规则全部以数据形式入库（症状谓词 → 真因 → 处置 → 出处）。
 * `when: null` 表示该条暂无可观测谓词（知识库条目，不触发卡片）——补谓词即生效，不动结构。
 * 引擎只跑数据，不加规则不改代码（文档08 诊断引擎扩容承诺）。
 */

export type DiagSeverity = "block" | "warn" | "info";

/** 诊断上下文：各 store 状态 + 信号文本池的快照 */
export interface DiagContext {
  doctor: DoctorReport | null;
  session: SessionSnapshot | null;
  serverStatus: ServerStatusReport | null;
  probes: ProbeStat[];
  injection: InjectionReport | null;
  /** 信号文本池：session evidence / 消息流 / doctor evidence / probe lastError / matrix */
  signals: string[];
  /** B1 主动探测发现（30s 巡检：session-drift 等），与被动信号分离 */
  activeFindings: string[];
  /** active 且 hits=0 的探针 id → 首次观测时间（O-03 的 15s 判据） */
  zeroHitSince: Map<string, number>;
  /** waiting 态探针 id → 首次观测时间（P-05 的类加载时序判据） */
  waitingSince: Map<string, number>;
  /** 最近一次内置爆破自测失败（C-07） */
  bruteSelfTestFailed: boolean;
  /** 爆破引擎是否为生成 C 骨架（C-06 提示） */
  bruteEngineGenerateC: boolean;
  now: number;
}

export interface DiagCard {
  /** 卡片 id：规则 ID + 关联对象（探针 id 等），用于去重与忽略 */
  id: string;
  ruleId: string;
  title: string;
  severity: DiagSeverity;
  cause: string;
  fix: string[];
  source: string;
}

interface Rule {
  id: string;
  title: string;
  severity: DiagSeverity;
  cause: string;
  fix: string[];
  source: string;
  when: ((ctx: DiagContext) => boolean) | null;
}

// ---------------- 谓词辅助 ----------------

const chk = (ctx: DiagContext, id: string) => ctx.doctor?.checks.find((c) => c.id === id);
const chkFail = (ctx: DiagContext, id: string) => chk(ctx, id)?.status === "fail";
const chkWarn = (ctx: DiagContext, id: string) => chk(ctx, id)?.status === "warn";
const hasSignal = (ctx: DiagContext, re: RegExp) => ctx.signals.some((s) => re.test(s));
const anyProbeError = (ctx: DiagContext, re: RegExp) =>
  ctx.probes.some((p) => p.lastError != null && re.test(p.lastError));

// ---------------- 48 条规则（docs/05-诊断知识库） ----------------

export const RULES: Rule[] = [
  // ===== E · 环境与连接（8） =====
  {
    id: "E-01",
    title: "adb devices 没有任何设备",
    severity: "block",
    cause: "模拟器没启动、未开调试，或 adb 端口不在候选列表里。",
    fix: ["① 确认模拟器已启动", "② 深度体检会自动扫 16384/16385/7555 等候选端口", "③ 仍失败则逐个人工试连"],
    source: "docs/05 E-01 · 实战：MuMu 端口漂移",
    when: (c) => chkFail(c, "CHK-01"),
  },
  {
    id: "E-02",
    title: "adb connect 长时间无响应",
    severity: "block",
    cause: "连到了非 adbd 端口（TCP 通但没有协议应答），不加超时会无限挂死。",
    fix: ["等 15s 硬超时自动结束（E-02 约束）", "换下一个候选端口重试", "失败报告会列出试过的端口与各自结果"],
    source: "docs/05 E-02 · 实战：connect 挂死一次排查 40 分钟",
    when: (c) => chkFail(c, "CHK-03") && hasSignal(c, /超时|timeout|timed out/i),
  },
  {
    id: "E-03",
    title: "设备显示 offline",
    severity: "warn",
    cause: "模拟器刚启动时 adbd 未就绪，首次 connect 会得到 offline。",
    fix: ["工具已自动 disconnect→2s→重连，最多 5 次", "全部失败才报错；可重启模拟器后再试"],
    source: "docs/05 E-03 · 实战：模拟器冷启动 offline",
    when: (c) => chkFail(c, "CHK-03") && hasSignal(c, /offline/i),
  },
  {
    id: "E-04",
    title: "找不到可用的 adb 或版本不匹配",
    severity: "block",
    cause: "系统 PATH 上的 adb 过老，与模拟器 adbd 协议不匹配。",
    fix: ["优先用模拟器自带的 adb.exe（MuMuPlayer-12.0\\shell\\adb.exe）", "在「设置」里指定自定义 adb 路径", "体检结果里可展开候选清单与来源"],
    source: "docs/05 E-04 · 实战：系统 adb 与 MuMu adbd 不兼容",
    when: (c) => chkFail(c, "CHK-02"),
  },
  {
    id: "E-05",
    title: "error: more than one device/emulator",
    severity: "warn",
    cause: "同时连了多台设备，裸 adb 命令不知道发给谁。",
    fix: ["一律带 -s <serial> 执行", "UI 里选中哪台，等价命令就带哪台的 serial"],
    source: "docs/05 E-05",
    when: null,
  },
  {
    id: "E-06",
    title: "设备没有 root 权限",
    severity: "block",
    cause: "adbd 未提权，回灌/frida-server 安装链都需要 root。",
    fix: ["执行 adb root（会重启 adbd，等 3s 重连）", "仍不行检查模拟器设置里是否开启 ROOT"],
    source: "docs/05 E-06",
    when: (c) => chkFail(c, "CHK-04"),
  },
  {
    id: "E-07",
    title: "SELinux Enforcing：回灌后 App 读不到文件",
    severity: "warn",
    cause: "SELinux 标签不对；Enforcing 模式下 restorecon 是必做步骤，不是可选项。",
    fix: ["回灌向导第⑥步已自动 restorecon（Enforcing 时）", "老 Android 无 restorecon 属正常，不阻断"],
    source: "docs/05 E-07 · 实战：灌完数据 App 读不到",
    when: (c) => chkWarn(c, "CHK-05") || hasSignal(c, /restorecon: not found/i),
  },
  {
    id: "E-08",
    title: "frida-server 启动即崩（Exec format error）",
    severity: "block",
    cause: "ABI 不匹配：给 x86_64 设备推了 arm64 包（或反之）。",
    fix: ["先 getprop ro.product.cpu.abi 确认设备 ABI", "按 ABI 选 frida-server 包（x86_64 / arm64-v8a）", "体检 CHK-06 会显示设备 ABI 与对应包名"],
    source: "docs/05 E-08",
    when: (c) => chkFail(c, "CHK-06") || hasSignal(c, /Exec format error/i),
  },

  // ===== S · 会话（7） =====
  {
    id: "S-01",
    title: "连不上 remote frida-server：客户端与设备端版本不一致",
    severity: "block",
    cause: "本机 frida 客户端与设备端 frida-server 版本不同（三处一致约束被破坏：客户端/内置矩阵/设备端）。",
    fix: ["查看会话控制台的版本三处一致矩阵", "推送与客户端匹配的 frida-server 版本（安装链自动做）", "重启 server 后重试附加"],
    source: "docs/05 S-01 · S-01 三处一致",
    when: (c) => {
      const s = c.serverStatus;
      if (hasSignal(c, /failed to connect to remote frida-server|unable to connect to remote frida-server/i)) return true;
      if (s?.client_version && s?.device_server_version && s.client_version !== s.device_server_version) return true;
      return chkFail(c, "CHK-07") || chkFail(c, "CHK-08");
    },
  },
  {
    id: "S-02",
    title: "frida-server 在跑但连接被拒：缺 adb forward",
    severity: "block",
    cause: "frida-server 监听在设备侧，主机访问必须先建 adb forward 端口映射。",
    fix: ["执行 adb -s <serial> forward tcp:27042 tcp:27042", "用 adb forward --list 确认映射存在", "会话控制台「建立 forward」按钮一键完成"],
    source: "docs/05 S-02",
    when: (c) => c.serverStatus?.server_running === true && c.serverStatus?.forward_established === false,
  },
  {
    id: "S-03",
    title: "frida-server 跑一会儿就断",
    severity: "warn",
    cause: "server 是前台进程，adb shell 通道断开（窗口关闭）就被 SIGHUP 杀掉；或会话在无事件通知的情况下静默死亡（主动探测发现）。",
    fix: ["安装链已用 nohup 后台化启动", "断连后重新执行「安装并启动」（幂等，S-03）", "检查是否有别的工具在重启 adb server"],
    source: "docs/05 S-03 · 实战：关掉终端窗口 server 即死",
    when: (c) =>
      hasSignal(c, /"event":"detached"|connectionTerminated|device_lost/i) ||
      c.activeFindings.includes("session-drift"),
  },
  {
    id: "S-04",
    title: "设备端有残留 frida-server，端口冲突起不来",
    severity: "warn",
    cause: "旧的 frida-server 进程还占着 27042，新实例绑定失败。",
    fix: ["安装链已内置 pkill -f frida-server 清理（等 1s 再起）", "手动清：adb shell su -c 'pkill -f frida-server'"],
    source: "docs/05 S-04",
    when: (c) => hasSignal(c, /pkill|残留|清理.*frida/i),
  },
  {
    id: "S-05",
    title: "★ 假绿灯：进程起了但没在监听",
    severity: "block",
    cause: "「启动命令返回成功」≠「server 在监听」——可能已崩溃或端口被改。",
    fix: ["独立验证：adb shell ss -tlnp | grep :27042", "状态灯以实测监听为准（声称与实测分开表示）", "用体检深度模式复核"],
    source: "docs/05 S-05 · 核心纪律",
    when: (c) =>
      c.serverStatus?.server_running === true &&
      c.serverStatus?.device_server_present === false,
  },
  {
    id: "S-06",
    title: "主机端口被占用，forward 已自动换端口",
    severity: "info",
    cause: "本机 27042 被其它程序占用，工具按 S-06 自动探测备用端口。",
    fix: ["无需处理：forward 已换用备用端口", "证据里可查实际使用的主机端口"],
    source: "docs/05 S-06 · 实战：本机端口冲突",
    when: (c) => hasSignal(c, /被占|占用.*换|换用.*端口|备用端口/),
  },
  {
    id: "S-07",
    title: "spawn 模式 App 卡白屏",
    severity: "warn",
    cause: "spawn 挂起等待注入，注入脚本报错会让进程一直停着。",
    fix: ["改用 attach 模式（附加已运行进程）", "或在深度脚本里修正报错脚本", "确认目标进程确实存在"],
    source: "docs/05 S-07",
    when: null,
  },

  // ===== P · 挂钩与类加载（8） =====
  {
    id: "P-01",
    title: "Java.use 抛 ClassNotFoundException",
    severity: "block",
    cause: "类名错（混淆/内嵌类分隔符/类在别的 dex）。",
    fix: ["用探索器（类搜索）核对真实类名", "内嵌类用 Outer$Inner 写法", "确认目标 dex 已被加载（加固壳先触发解壳）"],
    source: "docs/05 P-01",
    when: (c) => anyProbeError(c, /ClassNotFoundException|NoClassDefFoundError/),
  },
  {
    id: "P-02",
    title: "方法在类上不存在（undefined）",
    severity: "block",
    cause: "方法名错，或该方法是从父类继承的（不在本类声明里）。",
    fix: ["用「查看方法」打印类全部声明方法核对", "继承方法要沿父类链向上找"],
    source: "docs/05 P-02",
    when: (c) => anyProbeError(c, /method .*undefined|方法不存在|not a function/i),
  },
  {
    id: "P-03",
    title: "implementation 赋值报重载歧义",
    severity: "warn",
    cause: "同名方法有多个签名，直接赋值无法确定目标。",
    fix: ["工具的探针默认全重载展开（无需手写 overloads.forEach）", "自写脚本用 C.method.overloads.forEach(...)"],
    source: "docs/05 P-03",
    when: (c) => anyProbeError(c, /overload|重载/i),
  },
  {
    id: "P-04",
    title: "构造函数挂不上",
    severity: "warn",
    cause: "frida 里构造函数叫 $init，不是 <init> 或类名。",
    fix: ["用 C.$init.overloads.forEach(...) 挂构造", "对象建成后立即 dump 字段"],
    source: "docs/05 P-04",
    when: (c) => anyProbeError(c, /\$init|<init>|构造/i),
  },
  {
    id: "P-05",
    title: "探针长期 waiting：类还没被加载（17.x 时序）",
    severity: "warn",
    cause: "脚本跑得比类加载快；frida 17.x 下尤为明显。工具会自动延迟注册重试。",
    fix: ["进入会触发目标类的页面（登录页/设置页）", "探针三态里的 waiting 会在类加载后自动转 active", "加固壳类用 REPL 切换 classFactory.loader"],
    source: "docs/05 P-05 · 实战：17.x 静默无输出",
    when: (c) => {
      for (const p of c.probes) {
        if (p.status === "waiting") {
          const since = c.waitingSince.get(p.id);
          if (since != null && c.now - since > 10_000) return true;
        }
      }
      return false;
    },
  },
  {
    id: "P-06",
    title: "探针挂载失败（三态上报：error）",
    severity: "block",
    cause: "探针注册抛错——常见是类名/方法名问题或加固壳换加载器。",
    fix: ["看卡片真因里的具体错误文本", "在探索器确认类名/方法名存在", "加固壳：REPL 用 javaLoaders + useLoader 切换加载器"],
    source: "docs/05 P-06 · 三态上报机制",
    when: (c) => {
      const bad = c.probes.filter((p) => p.status === "error");
      return bad.length > 0;
    },
  },
  {
    id: "P-07",
    title: "Java.perform 代码没执行：目标可能无 Java 层",
    severity: "warn",
    cause: "目标不是 Java 层进程（纯 native），或 Java 桥不可用。",
    fix: ["hello 握手里 java 字段为 null 即无 Java VM", "纯 native 目标改用 native 探针（模块/导出）"],
    source: "docs/05 P-07",
    when: (c) =>
      c.session?.phase === "running" &&
      c.session?.hello != null &&
      (c.session.hello as Record<string, unknown>).java == null &&
      c.probes.length > 0,
  },
  {
    id: "P-08",
    title: "注入后进程立刻退出：疑似反调试",
    severity: "block",
    cause: "反调试/反 frida 检测——这是对手的领域，工具不承诺绕过，但会明确告诉你原因。",
    fix: ["特征确认：注入后进程立刻消失且无异常日志", "属对抗范畴，本工具只做明确报因（P-08 决策 A）", "可尝试 attach 而非 spawn、或延后注入时机"],
    source: "docs/05 P-08 · 决策 A：不内置绕过",
    when: (c) =>
      c.session?.phase === "stopped" &&
      hasSignal(c, /"event":"detached"/) &&
      c.probes.every((p) => p.hits === 0),
  },

  // ===== O · 观测与数据呈现（5） =====
  {
    id: "O-01",
    title: "参数打印成 [B@1a2b3c（byte[] 未格式化）",
    severity: "info",
    cause: "没有对 byte[]/对象做格式化，输出了默认 toString。",
    fix: ["探针表单的参数格式化器选 byte[]→hex（自动截断）", "这是填表选项，不该写代码"],
    source: "docs/05 O-01",
    when: null,
  },
  {
    id: "O-02",
    title: "日志刷屏、UI 卡顿",
    severity: "warn",
    cause: "大对象/高频调用未经截断，原始输出全量进 UI。",
    fix: ["格式化器自带长度上限（默认 128，超限显示 …(+N)）", "时间轴做虚拟滚动；原始输出按需下钻"],
    source: "docs/05 O-02",
    when: null,
  },
  {
    id: "O-03",
    title: "★ 探针装载成功但命中 0 次",
    severity: "warn",
    cause: "目标逻辑没被执行——最常见是 App 带着登录态直接进主页，根本没走登录分支。",
    fix: [
      "① 检查 prefs/配置里的登录态键，回灌时清掉（向导会预警）",
      "② 确认入口手势走对（隐藏手势见应用档案 entryGesture）",
      "③ 确认附加的是正确进程（spawn/attach 目标核对）",
    ],
    source: "docs/05 O-03 · 实战：登录态导致零命中",
    when: (c) => {
      for (const p of c.probes) {
        if (p.status === "active" && p.hits === 0) {
          const since = c.zeroHitSince.get(p.id);
          if (since != null && c.now - since > 15_000) return true;
        }
      }
      return false;
    },
  },
  {
    id: "O-04",
    title: "找不到某个功能入口（隐藏手势）",
    severity: "info",
    cause: "入口是长按 1.2s 才触发的隐藏手势，单击/双击都无反应；Flutter 界面无可枚举控件。",
    fix: ["依次试：单击 → 双击 → 长按(1s/1.2s/2s) → 三连点 → 角落坐标", "成功后把坐标记进应用档案（entryCoords）"],
    source: "docs/05 O-04 · 实战：长按齿轮 1.2s",
    when: null,
  },
  {
    id: "O-05",
    title: "只有纯文本日志无法比对",
    severity: "info",
    cause: "用 console.log 拼字符串丢掉了类型信息。",
    fix: ["结构化输出：agent send() JSON（本工具的默认行为）", "宿主不做文本正则（02 章 §3.2 核心决策）"],
    source: "docs/05 O-05",
    when: null,
  },

  // ===== D · 数据回灌（7） =====
  {
    id: "D-01",
    title: "adb push: Permission denied",
    severity: "block",
    cause: "adbd 非 root，或目标目录属主/权限不符。",
    fix: ["① 先 adb root 重连", "② 走中转：push 到 /data/local/tmp/ → su -c cp && chown（向导已内置）"],
    source: "docs/05 D-01",
    when: (c) => hasSignal(c, /push.*Permission denied|Permission denied.*push/i),
  },
  {
    id: "D-02",
    title: "su: invalid option -- a",
    severity: "block",
    cause: "su -c \"…\" 用双引号被 shell 拆包了。",
    fix: ["只能单引号：su -c 'cmd1; cmd2'", "工具的命令构造层已统一处理"],
    source: "docs/05 D-02",
    when: (c) => hasSignal(c, /invalid option/i),
  },
  {
    id: "D-03",
    title: "回灌后 App 启动闪退：chown 的 uid 错了",
    severity: "block",
    cause: "chown 的 uid 填错或目录属主不对。",
    fix: ["从 pm list packages -U 取真实 uid（向导第⑤步自动 stat 实测）", "重新 chown -R <uid>:<uid>"],
    source: "docs/05 D-03 · 实战：uid 10042/10043",
    when: (c) => c.injection?.overall === "fail" && hasSignal(c, /chown/i),
  },
  {
    id: "D-04",
    title: "restorecon: not found",
    severity: "info",
    cause: "老 Android 没有这个命令，非阻断。",
    fix: ["getenforce 是 Permissive 时可忽略", "Enforcing 且无 restorecon 需换方式（查 SELinux 上下文工具）"],
    source: "docs/05 D-04",
    when: (c) => hasSignal(c, /restorecon: not found/i),
  },
  {
    id: "D-05",
    title: "文件在但 App 当没有：md5 校验或标签不一致",
    severity: "warn",
    cause: "SELinux 标签未刷/属主不对/App 有完整性校验。",
    fix: ["先 restorecon -R（向导第⑥步）", "再核对 uid（第⑤步）", "仍不行怀疑 App 有校验，转静态分析"],
    source: "docs/05 D-05",
    when: (c) =>
      c.injection != null &&
      c.injection.steps.some((s) => s.name.includes("md5") && s.status === "fail"),
  },
  {
    id: "D-06",
    title: "回灌没有先停应用：内容被改回去了",
    severity: "warn",
    cause: "App 正在运行时会写回自己的状态，覆盖灌入的内容。",
    fix: ["回灌前必须 am force-stop <pkg>", "向导把「停应用」设为第①步不可跳过"],
    source: "docs/05 D-06",
    when: (c) =>
      c.injection != null &&
      c.injection.steps.some((s) => s.name.includes("force-stop") || s.name.includes("停应用")) &&
      c.injection.steps[0]?.status === "skip",
  },
  {
    id: "D-07",
    title: "mkdir 建的目录属主是 root",
    severity: "warn",
    cause: "目录是手工建的，属主 root，App 无权写。",
    fix: ["先空跑一次让 App 自己把目录结构建出来，再覆盖内容（向导第②步）", "第④.5 步会预警 shared_prefs 登录态键"],
    source: "docs/05 D-07",
    when: (c) =>
      c.injection != null &&
      c.injection.steps.some((s) => s.status === "skip" && s.name.includes("空跑")),
  },

  // ===== C · 加密库与算法（8） =====
  {
    id: "C-01",
    title: "DB Browser 报 file is not a database 且不弹密码框",
    severity: "info",
    cause: "文件不是明文 SQLite——头 16 字节是随机盐（SQLCipher）。",
    fix: ["读头 16 字节判定：53 51 4c 69 74 65… = 明文 SQLite", "否则走 SQLCipher 路径（需要口令）"],
    source: "docs/05 C-01",
    when: null,
  },
  {
    id: "C-02",
    title: "sqlcipher 命令行用同一口令打不开",
    severity: "info",
    cause: "CLI 与 python 绑定的 PRAGMA 处理路径不同。",
    fix: ["用 python 绑定（sqlcipher3）", "CLI 这条路不要再试"],
    source: "docs/05 C-02",
    when: null,
  },
  {
    id: "C-03",
    title: "口令对但开不了库：兼容档不同",
    severity: "info",
    cause: "SQLCipher 兼容档（KDF 迭代/算法/页大小）与生成时不一致。",
    fix: ["穷举组合：cipher_compatibility=3/4 + kdf_iter(64000/256000) + hmac/kdf 算法"],
    source: "docs/05 C-03",
    when: null,
  },
  {
    id: "C-04",
    title: "☠ 库一被写就重新加密",
    severity: "block",
    cause: "SQLCipher 库一被写就重新加密；或工具写坏了检材。",
    fix: ["只读挂载（:ro）或先复制到工作区", "本工具的只读保护是拒绝写，不是警告"],
    source: "docs/05 C-04 · 事故出处",
    when: null,
  },
  {
    id: "C-05",
    title: "找不到库口令",
    severity: "info",
    cause: "口令存放位置因 App 而异，且可能是编码值。",
    fix: ["已知形态①：shared_prefs/*.xml（可能明文）", "已知形态②：app_flutter/files/password.json（可能要 [1:-2] 变换）", "仍找不到：挂钩 SQLiteDatabase.$init 抓口令"],
    source: "docs/05 C-05",
    when: null,
  },
  {
    id: "C-06",
    title: "hashcat 无此模式：已生成 C 专用爆破器",
    severity: "info",
    cause: "算法是非标准组合（如链式十六进制 SHA-256 迭代 10000 轮），通用爆破器没有对应 mode。",
    fix: ["用生成的 C 骨架：gcc -O3 -march=native -fopenmp 编译", "骨架已含链式轮优化 + 强制自测桩", "先自测后全量（C-07）"],
    source: "docs/05 C-06 · writeup hcbrute4",
    when: (c) => c.bruteEngineGenerateC,
  },
  {
    id: "C-07",
    title: "★ 自测失败：拒绝全量爆破",
    severity: "block",
    cause: "自写实现与实际算法不等价（差一个编码层/一次迭代），全量跑必然白跑。",
    fix: ["用已知 (明文, 目标值) 先自测逐字节比对", "不过就改实现，绝不进入全量爆破", "内置爆破的强制自测门已拦截本次请求"],
    source: "docs/05 C-07 · C-07 硬门槛",
    when: (c) => c.bruteSelfTestFailed,
  },
  {
    id: "C-08",
    title: "明文算得出哈希但开不了库：编码层数不同",
    severity: "info",
    cause: "编码链层数不同（hex 几层/base64/原始字节）。",
    fix: ["逐层列出编码链", "分别验证「数学等价」与「功能可用」两条（原则 5 双证据）"],
    source: "docs/05 C-08",
    when: null,
  },

  // ===== X · 邻域（5） =====
  {
    id: "X-01",
    title: "DB 文件不是 SQLite 也不是 SQLCipher：可能是 bbolt",
    severity: "info",
    cause: "bbolt（etcd 存储）魔数 ED 0C ED DA，键值明文。",
    fix: ["strings -a -n 8 db 直接看", "要 GUI 用 boltbrowser"],
    source: "docs/05 X-01",
    when: null,
  },
  {
    id: "X-02",
    title: "strings 输出丢失中文",
    severity: "info",
    cause: "strings 只输出 ASCII 可打印字符，UTF-8 中文一个字节都不留且不报错。",
    fix: ["按字节无损读：Latin-1 转字符串后正则，再按 UTF-8 还原"],
    source: "docs/05 X-02",
    when: null,
  },
  {
    id: "X-03",
    title: "PowerShell > 重定向后文件体积翻倍",
    severity: "info",
    cause: "PS 5.1 的 > 默认写 UTF-16。",
    fix: ["用 Set-Content -Encoding UTF8 / Out-File -Encoding utf8"],
    source: "docs/05 X-03",
    when: null,
  },
  {
    id: "X-04",
    title: "MySQL 8.0 拒绝启动（MY-010159 系）",
    severity: "info",
    cause: "datadir 在 NTFS/9p 上，lower_case_table_names 探测成 2 与数据字典冲突——不是版本问题。",
    fix: ["datadir 搬进 Linux 端卷", "别浪费时间换版本"],
    source: "docs/05 X-04",
    when: null,
  },
  {
    id: "X-05",
    title: "MySQL 8.0 SHOW TABLES 返回空集不报错",
    severity: "info",
    cause: "8.0 起 .frm 元数据并入 InnoDB 数据字典，静默忽略 5.x MyISAM 表。",
    fix: ["换 mysql:5.7", "记住：没有报错 ≠ 没有问题"],
    source: "docs/05 X-05",
    when: null,
  },
];

const SEVERITY_ORDER: Record<DiagSeverity, number> = { block: 0, warn: 1, info: 2 };

/** 纯函数求值：按严重度排序输出卡片（同规则多探针只出一张卡，id 携带对象键） */
export function evaluateRules(ctx: DiagContext): DiagCard[] {
  const cards: DiagCard[] = [];
  for (const rule of RULES) {
    if (!rule.when || !rule.when(ctx)) continue;
    cards.push({
      id: rule.id,
      ruleId: rule.id,
      title: rule.title,
      severity: rule.severity,
      cause: rule.cause,
      fix: rule.fix,
      source: rule.source,
    });
    // 探针级卡片：为每个出错探针补一张带探针 id 的卡（在 probe 卡之外，供下钻）
    if (rule.id === "P-01" || rule.id === "P-02" || rule.id === "P-03" || rule.id === "P-04") {
      for (const p of ctx.probes) {
        if (p.lastError && rule.title && p.status === "error") {
          cards.push({
            id: `${rule.id}:${p.id}`,
            ruleId: rule.id,
            title: `${rule.title} → ${p.clazz}.${p.method}`,
            severity: rule.severity,
            cause: p.lastError,
            fix: rule.fix,
            source: rule.source,
          });
        }
      }
    }
  }
  return cards.sort(
    (a, b) => SEVERITY_ORDER[a.severity] - SEVERITY_ORDER[b.severity] || a.ruleId.localeCompare(b.ruleId),
  );
}
