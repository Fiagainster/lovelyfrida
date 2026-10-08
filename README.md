# lovelyfrida

> 把 Frida 动态分析从「命令行的手艺」变成「看得见、可回放、能自证的工作台」。

**当前状态：M0~M5 核心全部落地并真机验收（外壳/体检/只读保护 → frida 三通道/会话/终端/Recorder → agent 通用底座/探针/探索器/REPL/时间轴 → 回灌七步/实验台/差分矩阵 → 算法还原/爆破编排 → Evidence 台账/案卷包）；完善计划四阶段全部完成（P0 真 bug 清零 + 安全纪律兑现 → 诊断引擎 48 条数据驱动 + 爆破掩码循环 + 落库迁移 → 通道C/视图零占位 → 单安装包分发）。2026-10 优化批次三~六 + 批次七真机联调（MuMu x86_64 全业务链打通：U4 真案还原/U5 爆破 HIT/击杀自愈验收通过；联调揪出并修复 sidecar 假死、GBK 输出、同方法位探针顶掉等真 bug；验收脚本见 scripts/it_layer1_sidecar.py 与 it_layer2_gui.py，施工记录见 docs/10 阶段⑤）。**

---

## 一、它是什么

一个 **Tauri 桌面应用**（Rust 后端 + Vue 3 前端），把一次动态分析的完整链路做成一条有状态灯的流水线：

```
环境体检 → 设备连接 → 应用装载 → 数据回灌 → 探针注入 → 观测 → 受控实验 → 算法还原 → 爆破编排 → 证据归档
```

三条硬约束贯穿始终：

1. **每个可视操作都能下钻到等价的 adb / frida 原始命令** —— 工具教你原理，不是替你跳过原理。
2. **只读是默认值** —— 检材路径在工具里就是只读根，要写就先复制。
3. **结论必须双证据** —— 数学自证 + 真机复现，两者都过才算闭环。

## 二、技术栈与架构

| 层 | 选型 |
|---|---|
| 应用外壳 | Tauri 2.x（单 exe 分发、Rust 后端进程控制） |
| 前端 | Vue 3 + Vite + TypeScript + Pinia + Naive UI；终端 xterm.js（编辑用 NInput textarea、时间轴用 NDataTable 虚拟滚动替代 Monaco/ECharts 这类重组件——按需只留 xterm） |
| 后端 | Rust（tokio 多线程），rusqlite(bundled)、tracing、portable-pty（M1） |
| Frida 对接 | **三通道**：A=Rust frida crate（远期）/ **B=sidecar JSON-RPC over stdio（主通道；打包态为内置 frida_bridge.exe，开发态回退 python）** / C=frida CLI 兜底（观测级降级，RPC 类操作明确报错）；auto 先 B、B 不可用降级 C，当前通道 UI 必须明示；结构化数据一律 agent `send()` JSON 回传，宿主不解析控制台文本 |
| Agent | 注入进程的常驻 JS（M1 握手；M2 按已批准的 [09 扩展方案](docs/09-通用调试工作台扩展.md) 升级为通用调试底座 + instruments） |

```
┌─ 前端 Vue3 ── 六视图 + 会话控制台 + 诊断卡片流 + ⌘K ─┐
│            ↑ Tauri commands / events（L1/L2/L3 三层事件）
├─ Rust 应用服务 ── Doctor / SessionMgr / ProbeLab(M2) / Experiment(M3) / Crypto(M4) / BruteOrch(M4) / Ledger(M5) / Diagnostics / Recorder
│            ↑ 能力 trait（所有设备/frida 操作必经此层）
├─ backends ── AdbBackend │ FridaBackend(三通道) │ FileBackend(只读保护) │ CryptoBackend
└─ 外部世界 ── adb.exe / frida-server(设备端) / python sidecar / 模拟器 / 检材文件系统
```

## 三、目录结构

```
docs/            需求与设计文档（01~10，见下方文档地图）
src/             Vue 3 前端（views 六视图、components、stores、api、diagnostics 规则引擎）
src-tauri/       Rust 后端
  src/services/  应用服务层（doctor / session / first_run / store / ledger …）
  src/backends/  能力层（adb / frida 通道B sidecar / frida_c 通道C）
  src/commands/  Tauri commands（按域分组）
agent/           注入 agent（src/ TypeScript 源码 → esbuild 产 dist/core.js，构建期内嵌 Rust）
sidecar/         通道B：frida_bridge.py（JSON-RPC over stdio）；dist/ 为打包产物（不入库）
scripts/         dev.mjs（自动选端口启动）、build_sidecar.py（PyInstaller 打包）、gen_icon.py
bin/             随包二进制（不入库，见下方「二进制供给」）+ binary_manifest.json（入库）
workspace/       运行时唯一可写区（不入库）
cases/ logs/     归档与审计日志（不入库）
```

## 四、开发指南

前置：Node ≥ 20、Rust stable (MSVC)、Python 3.11+ 且 `pip install frida`（本机 17.19.0）、可选 MuMu 模拟器。

```bash
npm install
npm run dev:tauri     # 自动探测可用端口（规避 WinNAT 保留段漂移）→ vite + tauri dev
npm run build         # vue-tsc + vite 生产构建
cd src-tauri && cargo check
```

- **首次克隆后先构建 agent**（`cd agent && npm install && npm run build`）：`agent/dist/core.js` 是 gitignore 的本地构建产物，Rust 侧 `include_str!` 编译期依赖它（CI 已按此顺序固化）。
- **真机联调/验收脚本**（需 MuMu 运行、root、frida-server 在位）：
  `python scripts/it_layer1_sidecar.py com.notevault.app <host_port>`（协议层 18 项）；
  `python scripts/it_stress_events.py`（O-02 压测：30000 事件双阶段，
  批量路径 5022 事件/s 达标、未批量对照基线）；
  `python scripts/it_layer2_gui.py`（CDP 驱动真实 UI 全业务链，需以
  `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9223 --remote-allow-origins=*"` 启动应用）。
  注意：混用不同版本的 adb CLI 与应用会触发 E-01/E-04 server 乒乓（详见 docs/10 阶段⑤坑位记录）。

- 端口被 WinNAT 间歇保留（EACCES）时 `dev.mjs` 会自动换端口，无需手动处理。
- 配置在首次运行生成 `config.toml`（含 `[doctor]` 体检可配置段、只读根、frida 端口）。
- 浏览器直开 vite（`npm run dev`）可做 UI 预览，Tauri 命令会返回明确的「预览模式不可用」。

### 打包分发（单安装包，阶段④）

```bash
python scripts/build_sidecar.py   # ① 通道B sidecar exe（PyInstaller onefile，捆绑 frida 客户端）
npx tauri build --config src-tauri/tauri.bundle.conf.json
#                                 ② NSIS 安装包（默认瘦身：随包 adb + sidecar，实测约 65MB）
# 随包资源清单独立在 bundle overlay：cargo check/test/clippy 不要求随包二进制存在（CI 免供给），
# 只有打包（tauri build）时才需要它们在位
```

**分发形态（C1，2026-10 拍板：默认瘦身）**：默认安装包**不随包 frida-server 矩阵**；首启后用体检页「下载 frida-server」按钮按 binary_manifest.json 登记的 sha256 对账下载到工作区（**首启需网络**，之后全功能离线）。需要「零联网、全量随包」的交付场景用：
`npx tauri build --config src-tauri/tauri.full.conf.json`（四 ABI 矩阵全随包，约 250MB；full overlay 自带完整资源清单）。

**签名（内部测试可用自签证书）**：`powershell -ExecutionPolicy Bypass -File scripts/sign_installer.ps1 <安装包>`；脚本头注释含证书准备（自签消除「无发布者」提示，正式分发用 CA 证书）。FR-02 首启对账对按需模式有明确容错（矩阵目录整体缺失时记「按需下载模式」而非失败）。

产物：`src-tauri/target/release/bundle/nsis/LovelyFrida_0.2.0_x64-setup.exe`。
安装后**零外部依赖**：adb 随包；frida-server 按设备 ABI 在运行时自动推送（S-01 版本矩阵一致）；通道B sidecar 为内置 exe（捆绑 Python + frida），开发态没有 exe 时自动回退 `python -u sidecar/frida_bridge.py`（需 `pip install frida`）。首启自检 FR-07 会明示当前 sidecar 启动方式。仅 frida 相关依赖在运行时由工具自行配置——这正是设计目标。

**签名（可选）**：正式分发前用 `scripts/sign_installer.ps1` 签安装包（SHA256 + RFC3161 时间戳），消除 SmartScreen「未知发布者」；证书准备方式见脚本头注释。

## 五、二进制供给（不入库）

`bin/` 下的可执行文件不入 git，按 `bin/binary_manifest.json`（入库）登记的 sha256 供给与校验：

```
bin/adb/windows-x64/{adb.exe,AdbWinApi.dll,AdbWinUsbApi.dll}     # platform-tools
bin/frida-server/<版本>/android-{arm,arm64,x86,x86_64}/frida-server
```

frida-server 从 [github.com/frida/frida/releases](https://github.com/frida/frida/releases) 下载，**版本必须与本机 frida 客户端一致**（S-01 三处一致）。首启自检（FR-02）按清单逐个校验 sha256。

打包时这些二进制作为 tauri resources 全量随包（NSIS 对父目录资源落 `_up_\` 前缀目录，`paths::resource_join` 两级定位），安装目录即绿色布局。

## 六、文档地图

| # | 文档 | 一句话 |
|:--:|---|---|
| 01 | [问题域与设计原则](docs/01-问题域与设计原则.md) | 33 条实战痛点台账 + 6 条设计原则 + 与现有工具的关系 |
| 02 | [总体架构](docs/02-总体架构.md) | 技术栈、分层、**Frida 三通道**、进程模型、打包分发 |
| 03 | [可视化交互设计](docs/03-可视化交互设计.md) | **六种视图** + 状态灯语义 + 下钻机制 + 线框 |
| 04 | [功能模块详设](docs/04-功能模块详设.md) | A~H 八个模块 + 三条横切能力 |
| 05 | [诊断知识库](docs/05-诊断知识库.md) | 症状 → 真因 → 处置 规则表（**48 条**，全部有实测出处） |
| 06 | [数据模型与存储](docs/06-数据模型与存储.md) | **16 个实体** + 磁盘布局 + trace 格式 + 交接 |
| 07 | [安全合规与工程纪律](docs/07-安全合规与工程纪律.md) | 只读、脱敏、审计、法律边界、供应链 |
| 08 | [路线图与验收标准](docs/08-路线图与验收标准.md) | M0~M5 + 10 个真实任务验收用例 + 风险与待决策项 |
| 09 | [通用调试工作台扩展](docs/09-通用调试工作台扩展.md) | **已批准**：M2 扩容——core agent 通用底座 + 探索器/REPL/脚本库 + 边界决策（不接 AI） |
| 10 | [完善计划](docs/10-完善计划.md) | M5 后全面体检：四阶段施工记录（P0 真 bug/安全兑现 → 诊断引擎/爆破/落库 → 通道C/视图 → 单安装包分发）+ 后续优化方向 |
里程碑进度（详见 [08 路线图](docs/08-路线图与验收标准.md)）：**M0 ✅ → M1 ✅ → M2 ✅ → M3 ✅ → M4 ✅ → M5 核心 ✅**。每阶段均经真机（MuMu x86_64）或单测验收，验收脚本见 scripts/。

## 七、非目标（明确不做）

| 不做 | 为什么 |
|---|---|
| MobSF 式「一键出报告」黑盒 | 本工具的全部价值在**每一步可见、可下钻**；黑盒与之相反 |
| root/越狱/反调试的**内置**自动化绕过 | 文档07 授权红线（决策 A：提供通用脚本运行能力，但不内置绕过模板） |
| 云端上传、遥测、多机并行 | 案件数据不出本机；`config.toml` 里没有 telemetry 这个开关 |
| 替代 jadx 等静态工具 | 它是**静态分析的下游**：静态告诉你「有个 `hashPassword`」，动态告诉它「到底吃什么参数」 |
| 打包成 Web 服务 | 动态分析的本质是本地进程控制；浏览器化只会引入一层无用的不确定 |
| **接入 AI / LLM** | 工具的价值 = 业务与能力的封装本身。需要 AI 的人直接用 frida 命令行即可；本工具把「每一步可见、可下钻、可重放」做到位，不在此之上再叠一层黑盒 |

## 八、一句话记住设计取向

**这个工具不是用来「少做几步」的，是用来「看懂每一步」。**

一次成功的动态分析里，真正稀缺的不是算力，是**知道现在卡在哪、以及为什么卡**。lovelyfrida 把「知道」这件事产品化。

---

仅用于自己有权处理的设备与应用（CTF 赛题、授权取证、自有应用）。使用即表示你理解并遵守当地法律法规。
