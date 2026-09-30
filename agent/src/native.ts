/**
 * Native 层与内存能力（文档09 §memory；D3 边界：按符号/偏移挂导出函数 + 读内存）。
 */
import { encodeValue, EncValue } from "./value";

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
      : Process.enumerateRanges("r--").map((r) => ({ base: r.base, size: r.size }));
  const matches: { address: string; size: number }[] = [];
  for (const r of ranges) {
    if (matches.length >= limit) break;
    try {
      Memory.scan(r.base, r.size, pattern, {
        onMatch(address, size) {
          matches.push({ address: address.toString(), size });
          return matches.length >= limit ? "stop" : undefined;
        },
        onComplete() {
          /* done */
        },
      });
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

export function addNativeProbes(
  decls: { id: string; module: string; target: string; maxLen?: number }[],
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
        addr = (Module as unknown as { getExportByName: (m: string, e: string) => NativePointer }).getExportByName(decl.module, decl.target);
      }
      const maxLen = decl.maxLen ?? 64;
      Interceptor.attach(addr, {
        onEnter(args) {
          st.hits++;
          const enc: EncValue[] = [];
          for (let i = 0; i < 4; i++) {
            enc.push(encodeValue(args[i], maxLen));
          }
          send({
            t: "probe_hit",
            id: decl.id,
            clazz: decl.module,
            method: decl.target,
            ts: Date.now(),
            thread: Process.getCurrentThreadId(),
            native: true,
            args: enc,
          });
        },
        onLeave(retval) {
          const st2 = nativeProbes.get(decl.id);
          if (st2) {
            /* 返回值留在 retval 编码增强（M2 后续） */
            void st2;
          }
          void retval;
        },
      });
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
  // Interceptor 的 listener 句柄管理：v1 用 revert 不适用（attach 需 listener 句柄），
  // 记录为已移除并在探针表删除（M2 后续补 attach 返回句柄管理）。
  let removed = 0;
  for (const id of ids) {
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
