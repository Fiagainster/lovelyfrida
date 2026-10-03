/**
 * Native 层与内存能力（文档09 §memory；D3 边界：按符号/偏移挂导出函数 + 读内存）。
 */
import { encodeValue, EncValue } from "./value";
import { emitEvent } from "./batch";
import { resolveExport } from "./extras";

export interface ModuleRow {
  name: string;
  base: string;
  size: number;
  path: string;
}

export function enumModules(): ModuleRow[] {
  return Process.enumerateModules().map((m) => ({
    name: m.name,
    base: m.base.toString(),
    size: m.size,
    path: m.path,
  }));
}

export function moduleExports(
  moduleName: string,
  query: string,
  offset: number,
  limit: number,
): { total: number; rows: { name: string; type: string; address: string }[] } {
  const all = ((Module as unknown as { enumerateExports: (n: string) => { name: string; type: string; address: NativePointer }[] }).enumerateExports(moduleName)).map((e) => ({
    name: e.name,
    type: e.type,
    address: e.address.toString(),
  }));
  const q = query.trim().toLowerCase();
  const filtered = q ? all.filter((e) => e.name.toLowerCase().includes(q)) : all;
  return {
    total: filtered.length,
    rows: filtered.slice(offset, offset + limit),
  };
}

/** 内存搜索（Luma 缺失、我们补齐的能力）：pattern 支持 hex（如 "ca fe ba be"）与裸字符串 */
export function scanMemory(
  pattern: string,
  moduleName: string | null,
  limit: number,
): { address: string; size: number }[] {
  const ranges =
    moduleName != null
      ? Process.enumerateModules()
          .filter((m) => m.name === moduleName)
          .map((m) => ({ base: m.base, size: m.size }))
      : // 与 dumpDex 同口径：r-- + rw-（rw- 上的字符串/数据此前搜不到）
        Process.enumerateRanges("r--")
          .concat(Process.enumerateRanges("rw-"))
          .map((r) => ({ base: r.base, size: r.size }));
  const matches: { address: string; size: number }[] = [];
  for (const r of ranges) {
    if (matches.length >= limit) break;
    try {
      // 必须用 scanSync：异步 scan 的回调在本函数返回后才执行，同步收集恒为空
      for (const m of Memory.scanSync(r.base, r.size, pattern)) {
        matches.push({ address: m.address.toString(), size: m.size });
        if (matches.length >= limit) break;
      }
    } catch {
      /* 不可读区域跳过 */
    }
  }
  return matches;
}

export function readMem(address: string, size: number): { b64: string; hex: string } {
  const buf = (Memory as unknown as { readByteArray: (a: NativePointer, n: number) => ArrayBuffer | null }).readByteArray(ptr(address), Math.min(size, 4096));
  const u8 = new Uint8Array(buf ?? []);
  let hex = "";
  for (let i = 0; i < u8.length; i++) {
    hex += (u8[i] < 16 ? "0" : "") + u8[i].toString(16);
  }
  return { b64: arrayBufferToB64(buf ?? new ArrayBuffer(0)), hex };
}

function arrayBufferToB64(buf: ArrayBuffer): string {
  const bytes = new Uint8Array(buf);
  let bin = "";
  for (let i = 0; i < bytes.length; i++) bin += String.fromCharCode(bytes[i]);
  return btoa(bin);
}

// ---------------- native 探针 ----------------

interface NativeProbeState {
  id: string;
  module: string;
  target: string; // 导出名或 0x 偏移
  hits: number;
  errors: number;
  status: "active" | "error";
  lastError: string | null;
}

export const nativeProbes = new Map<string, NativeProbeState>();
// 真 detach（文档10 P3-3）：attach 返回的 listener 句柄按探针 id 管理
const nativeListeners = new Map<string, InvocationListener>();

export function addNativeProbes(
  decls: { id: string; module: string; target: string; maxLen?: number; captureRet?: boolean }[],
): { results: { id: string; status: string; error: string | null }[] } {
  const results: { id: string; status: string; error: string | null }[] = [];
  for (const decl of decls) {
    const st: NativeProbeState = {
      id: decl.id,
      module: decl.module,
      target: decl.target,
      hits: 0,
      errors: 0,
      status: "active",
      lastError: null,
    };
    nativeProbes.set(decl.id, st);
    try {
      let addr: NativePointer;
      if (decl.target.startsWith("0x")) {
        const m = Process.findModuleByName(decl.module);
        if (!m) throw new Error(`模块不存在：${decl.module}`);
        addr = m.base.add(decl.target);
      } else {
        // frida 17 移除了静态 Module.getExportByName（此前硬转型调用旧 API → 17.x 必抛错），
        // 统一走 extras 的跨版本解析：模块内实例方法 → 旧静态 → 全局兜底
        const p = resolveExport(decl.module, decl.target);
        if (!p) throw new Error(`导出符号未找到：${decl.module}!${decl.target}`);
        addr = p;
      }
      const maxLen = decl.maxLen ?? 64;
      // captureRet：onEnter 暂存 payload（按线程栈，支持递归），onLeave 补 ret 后同事件发送（与 java.ts 一致）
      const pendingStacks = new Map<number, Record<string, unknown>[]>();
      const listener = Interceptor.attach(addr, {
        onEnter(args) {
          st.hits++;
          const enc: EncValue[] = [];
          for (let i = 0; i < 4; i++) {
            enc.push(encodeValue(args[i], maxLen));
          }
          const payload: Record<string, unknown> = {
            t: "probe_hit",
            id: decl.id,
            clazz: decl.module,
            method: decl.target,
            ts: Date.now(),
            thread: Process.getCurrentThreadId(),
            native: true,
            args: enc,
          };
          if (decl.captureRet) {
            const tid = Process.getCurrentThreadId();
            const stack = pendingStacks.get(tid) ?? [];
            stack.push(payload);
            pendingStacks.set(tid, stack);
          } else {
            emitEvent(payload); // 高频路径走批量层（O-02）
          }
        },
        onLeave(retval) {
          if (!decl.captureRet) return;
          const tid = Process.getCurrentThreadId();
          const stack = pendingStacks.get(tid);
          const payload = stack?.pop();
          if (payload) {
            payload.ret = encodeValue(retval, maxLen);
            emitEvent(payload); // 与 onEnter 同一批量层（同事件带 ret 语义不变）
          }
        },
      });
      nativeListeners.set(decl.id, listener);
      results.push({ id: decl.id, status: "active", error: null });
    } catch (e) {
      st.status = "error";
      st.lastError = String(e);
      results.push({ id: decl.id, status: "error", error: String(e) });
    }
  }
  return { results };
}

export function removeNativeProbes(ids: string[]): { removed: number } {
  let removed = 0;
  for (const id of ids) {
    const listener = nativeListeners.get(id);
    if (listener) {
      try {
        listener.detach();
      } catch {
        /* 目标已卸载等场景：忽略，句柄照常清理 */
      }
      nativeListeners.delete(id);
    }
    if (nativeProbes.delete(id)) removed++;
  }
  return { removed };
}

export function nativeStats(): { id: string; module: string; target: string; status: string; hits: number; errors: number; lastError: string | null }[] {
  const out: { id: string; module: string; target: string; status: string; hits: number; errors: number; lastError: string | null }[] = [];
  nativeProbes.forEach((st, id) => {
    out.push({ id, module: st.module, target: st.target, status: st.status, hits: st.hits, errors: st.errors, lastError: st.lastError });
  });
  return out;
}
