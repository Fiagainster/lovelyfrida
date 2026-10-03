// core agent — 每个被注入进程一个常驻实例（M1：握手与消息回路；M2 升级为通用调试底座）。
// 纪律（文档02）：一切结构化数据走 send() JSON，宿主不解析控制台文本。
//
// ★ 本文件与 agent/dist/core.js 的关系（A4d 澄清，勿混淆）：
//   - 生产注入的是 agent/dist/core.js —— 由 agent/src/index.ts（TypeScript，26 个
//     rpc.exports）经 esbuild 打包，构建期内嵌进 Rust 二进制（backends/frida.rs
//     的 include_str!，通道C 的 send-shim 包装也基于它）。
//   - 本文件是 M1 时期的手写最小版（仅 hello + ping/pong），只被
//     scripts/e2e_test.py 的「协议级」测试读取使用——它测的是 sidecar 协议本身，
//     不涉及 M2 agent 底座。改 agent 能力请去 agent/src/，改完 npm run build。

send({
  t: "hello",
  frida: Frida.version,
  runtime: Script.runtime,
  pid: Process.id,
  arch: Process.arch,
  platform: Process.platform,
  java: (typeof Java !== "undefined" && Java.available) ? Java.androidVersion : null,
});

// 消息回路：M1 只做回声自检（宿主 ping → pong），M2 探针/枚举指令在此扩展
recv("ping", function (message) {
  send({ t: "pong", echo: message.data || null });
  recv("ping", arguments.callee);
});
