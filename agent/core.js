// core agent — 每个被注入进程一个常驻实例（M1：握手与消息回路；M2 升级为通用调试底座）。
// 纪律（文档02）：一切结构化数据走 send() JSON，宿主不解析控制台文本。

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
