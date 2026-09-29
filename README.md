# lovelyfrida

> 把 Frida 动态分析从「命令行的手艺」变成「看得见、可回放、能自证的工作台」。

**当前状态：M0 已交付（外壳/体检/只读保护），M1 核心已落地（frida 通道B / frida-server 托管 / 会话状态机），下一步真机联调 + PTY 终端 + Recorder。**

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
| 前端 | Vue 3 + Vite + TypeScript + Pinia + Naive UI，编辑器 Monaco、终端 xterm.js、图表 ECharts |
| 后端 | Rust（tokio 多线程），rusqlite(bundled)、tracing、portable-pty（M1） |
| Frida 对接 | **三通道同一 trait**：A=Rust frida crate（远期）/ **B=Python sidecar JSON-RPC（主通道，已实现）** / C=CLI 兜底；结构化数据一律 agent `send()` JSON 回传，宿主不解析控制台文本 |
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
docs/            需求与设计文档（01~09，见下方文档地图）
src/             Vue 3 前端（views 六视图、components、stores、api）
src-tauri/       Rust 后端
  src/services/  应用服务层（doctor / session / first_run）
  src/backends/  能力层（adb / frida sidecar）
  src/commands/  Tauri commands（按域分组）
agent/           注入 agent（M1 core.js 握手 → M2 通用底座）
sidecar/         通道B：frida_bridge.py（JSON-RPC over stdio）
scripts/         dev.mjs（自动选端口启动）、gen_icon.py
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

- 端口被 WinNAT 间歇保留（EACCES）时 `dev.mjs` 会自动换端口，无需手动处理。
- 配置在首次运行生成 `config.toml`（含 `[doctor]` 体检可配置段、只读根、frida 端口）。
- 浏览器直开 vite（`npm run dev`）可做 UI 预览，Tauri 命令会返回明确的「预览模式不可用」。

## 五、二进制供给（不入库）

`bin/` 下的可执行文件不入 git，按 `bin/binary_manifest.json`（入库）登记的 sha256 供给与校验：

```
bin/adb/windows-x64/{adb.exe,AdbWinApi.dll,AdbWinUsbApi.dll}     # platform-tools
bin/frida-server/<版本>/android-{arm,arm64,x86,x86_64}/frida-server
```

frida-server 从 [github.com/frida/frida/releases](https://github.com/frida/frida/releases) 下载，**版本必须与本机 frida 客户端一致**（S-01 三处一致）。首启自检（FR-02）按清单逐个校验 sha256。

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
| 09 | [通用调试工作台扩展](docs/09-通用调试工作台扩展.md) | **已批准**：M2 扩容——core agent 通用底座 + 探索器/REPL/脚本库 + 边界决策 A |

里程碑进度详见 [08 路线图](docs/08-路线图与验收标准.md)：**M0 ✅ → M1 核心 ✅（联调中）→ M2（扩容方案已批准）**。

## 七、非目标（明确不做）

| 不做 | 为什么 |
|---|---|
| MobSF 式「一键出报告」黑盒 | 本工具的全部价值在**每一步可见、可下钻**；黑盒与之相反 |
| root/越狱/反调试的**内置**自动化绕过 | 文档07 授权红线（决策 A：提供通用脚本运行能力，但不内置绕过模板） |
| 云端上传、遥测、多机并行 | 案件数据不出本机；`config.toml` 里没有 telemetry 这个开关 |
| 替代 jadx 等静态工具 | 它是**静态分析的下游**：静态告诉你「有个 `hashPassword`」，动态告诉它「到底吃什么参数」 |
| 打包成 Web 服务 | 动态分析的本质是本地进程控制；浏览器化只会引入一层无用的不确定 |

## 八、一句话记住设计取向

**这个工具不是用来「少做几步」的，是用来「看懂每一步」。**

一次成功的动态分析里，真正稀缺的不是算力，是**知道现在卡在哪、以及为什么卡**。lovelyfrida 把「知道」这件事产品化。

---

仅用于自己有权处理的设备与应用（CTF 赛题、授权取证、自有应用）。使用即表示你理解并遵守当地法律法规。
