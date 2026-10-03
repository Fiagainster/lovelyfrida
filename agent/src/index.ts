/**
 * core agent 入口（文档09 通用调试底座）：
 * hello 握手 / console 猴子补丁分级 / rpc.exports（Java 枚举、探针、native、内存、REPL）。
 * 结构化数据一律 send() JSON（文档02 核心决策）。
 */
import Java from "frida-java-bridge";
import { encodeValue, EncValue } from "./value";
import {
  addProbes,
  chooseInstances,
  javaInfo,
  listClasses,
  listLoaders,
  listMethods,
  probeStats,
  removeProbes,
  searchMethods,
  useLoader,
} from "./java";
import {
  addNativeProbes,
  enumModules,
  moduleExports,
  nativeStats,
  readMem,
  removeNativeProbes,
  scanMemory,
} from "./native";
import { complete, evaluate } from "./repl";
import {
  chooseInstancesDetailed,
  dumpDex,
  invokeOnInstance,
  sslStats,
  watchDlopen,
  watchRegisterNatives,
  unwatchSsl,
  watchSsl,
} from "./extras";

// ---------------- console 猴子补丁（分级 send） ----------------

["log", "info", "warn", "error", "debug"].forEach((level) => {
  const orig = (console as unknown as Record<string, (...a: unknown[]) => void>)[level];
  (console as unknown as Record<string, (...a: unknown[]) => void>)[level] = (...args: unknown[]) => {
    orig(...args);
    const enc: EncValue[] = args.map((a) => encodeValue(a, 256, 2));
    send({ t: "console", level, args: enc } as unknown as { [key: string]: unknown });
  };
});

// ---------------- hello ----------------

function helloPayload(): Record<string, unknown> {
  const ji = javaInfo();
  return {
    t: "hello",
    frida: Frida.version,
    runtime: Script.runtime,
    pid: Process.id,
    arch: Process.arch,
    platform: Process.platform,
    java: ji.available ? ji.androidVersion : null,
  };
}

send(helloPayload());

// ---------------- rpc.exports ----------------

interface RpcExports {
  hello(): Record<string, unknown>;
  javaClasses(q: { query: string; limit: number; offset: number }): unknown;
  javaMethods(q: { className: string }): unknown;
  javaSearchMethods(q: { query: string; limit: number }): unknown;
  javaLoaders(): unknown;
  javaUseLoader(q: { index: number }): unknown;
  javaChoose(q: { className: string; limit: number }): Promise<unknown>;
  addProbes(q: { probes: unknown[] }): unknown;
  removeProbes(q: { ids: string[] }): unknown;
  probeStats(): unknown;
  addNativeProbes(q: { probes: unknown[] }): unknown;
  removeNativeProbes(q: { ids: string[] }): unknown;
  nativeStats(): unknown;
  modules(): unknown;
  moduleExports(q: { module: string; query: string; offset: number; limit: number }): unknown;
  scanMemory(q: { pattern: string; module: string | null; limit: number }): unknown;
  readMem(q: { address: string; size: number }): unknown;
  replEval(q: { code: string }): unknown;
  replComplete(q: { code: string; cursor: number }): unknown;
  watchDlopen(): unknown;
  watchRegisterNatives(): unknown;
  dumpDex(q: { maxDex: number }): unknown;
  watchSsl(q: { id: string; maxBuf: number }): unknown;
  unwatchSsl(q: { id: string }): unknown;
  sslStats(): unknown;
  chooseDetailed(q: { className: string; limit: number }): unknown;
  invokeInstance(q: { className: string; hashCode: number; methodName: string; args: string[] }): unknown;
}

const api: RpcExports = {
  hello: helloPayload,
  javaClasses: (q) => listClasses(q.query, q.limit, q.offset),
  javaMethods: (q) => listMethods(q.className),
  javaSearchMethods: (q) => searchMethods(q.query, q.limit),
  javaLoaders: () => listLoaders(),
  javaUseLoader: (q) => useLoader(q.index),
  javaChoose: (q) => chooseInstances(q.className, q.limit),
  addProbes: (q) => addProbes(q.probes as Parameters<typeof addProbes>[0]),
  removeProbes: (q) => removeProbes(q.ids),
  probeStats: () => probeStats(),
  addNativeProbes: (q) => addNativeProbes(q.probes as Parameters<typeof addNativeProbes>[0]),
  removeNativeProbes: (q) => removeNativeProbes(q.ids),
  nativeStats: () => nativeStats(),
  modules: () => enumModules(),
  moduleExports: (q) => moduleExports(q.module, q.query, q.offset, q.limit),
  scanMemory: (q) => scanMemory(q.pattern, q.module, q.limit),
  readMem: (q) => readMem(q.address, q.size),
  replEval: (q) => evaluate(q.code),
  replComplete: (q) => complete(q.code, q.cursor),
  watchDlopen: () => watchDlopen(),
  watchRegisterNatives: () => watchRegisterNatives(),
  dumpDex: (q) => dumpDex(q.maxDex),
  watchSsl: (q) => watchSsl(q.id, q.maxBuf),
  unwatchSsl: (q) => unwatchSsl(q.id),
  sslStats: () => sslStats(),
  chooseDetailed: (q) => chooseInstancesDetailed(q.className, q.limit),
  invokeInstance: (q) => invokeOnInstance(q.className, q.hashCode, q.methodName, q.args),
};

rpc.exports = api as unknown as Record<string, unknown>;

// ping 回路保留（M1 自检用）。recv 是一次性注册，须在回调里重新挂；
// 不能用 arguments.callee（esbuild 产物为严格模式，访问即抛 TypeError，第二个 ping 起失联）
function onPing(message: unknown): void {
  const data = (message as { data?: unknown })?.data ?? null;
  send({ t: "pong", echo: data });
  recv("ping", onPing);
}
recv("ping", onPing);

// REPL 需要 Java 全局可见（模块作用域内 import，挂到 globalThis 供 eval 使用）
if (Java.available) {
  (globalThis as unknown as { Java: unknown }).Java = Java;
}

// Java 可用时延迟一次 runtime 信息（17.x 时序：Java VM 可能晚于脚本就绪）
if (Java.available) {
  Java.perform(() => {
    send({ t: "java_ready", androidVersion: Java.androidVersion } as unknown as { [key: string]: unknown });
  });
}
