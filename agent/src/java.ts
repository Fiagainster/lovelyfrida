/**
 * Java 层能力（文档09 §enum/hooks）：
 * - 类/方法枚举搜索（Java.enumerateLoadedClassesSync / Java.enumerateMethods）
 * - 类加载器列表与切换（加固壳场景，P-05）
 * - 实例搜索（Java.choose）
 * - 声明式探针挂载：重载全展开、$init、byte[] hex、截断、三态上报、命中计数（O-01/O-02/O-03）
 */
import Java from "frida-java-bridge";
import { encodeValue, EncValue } from "./value";

export function javaAvailable(): boolean {
  try {
    return Java.available;
  } catch {
    return false;
  }
}

export function javaInfo(): { available: boolean; androidVersion: string | null; factories: number } {
  if (!javaAvailable()) return { available: false, androidVersion: null, factories: 0 };
  let v: string | null = null;
  try {
    v = Java.androidVersion;
  } catch {
    v = null;
  }
  return { available: true, androidVersion: v, factories: 1 };
}

function performSync<T>(fn: () => T): T {
  let out: T;
  let err: unknown = null;
  Java.perform(() => {
    try {
      out = fn();
    } catch (e) {
      err = e;
    }
  });
  if (err !== null) throw err;
  return out!;
}

// ---------------- 类枚举 ----------------

export function listClasses(query: string, limit: number, offset: number): {
  total: number;
  rows: { name: string }[];
} {
  return performSync(() => {
    const all = Java.enumerateLoadedClassesSync();
    const q = query.trim().toLowerCase();
    const filtered = q ? all.filter((n: string) => n.toLowerCase().includes(q)) : all;
    const total = filtered.length;
    const rows = filtered
      .slice(offset, offset + limit)
      .map((name: string) => ({ name }));
    return { total, rows };
  });
}

export function searchMethods(query: string, limit: number): {
  total: number;
  rows: { className: string; methodName: string; isStatic: boolean | null }[];
} {
  return performSync(() => {
    // Java.enumerateMethods(query)：query 形如 "*class!*method"
    const groups = Java.enumerateMethods(query) as unknown as Array<{
      loader: unknown;
      classes: Array<{ name: string; methods: Array<{ name: string; isStatic?: boolean }> }>;
    }>;
    const rows: { className: string; methodName: string; isStatic: boolean | null }[] = [];
    for (const g of groups) {
      for (const c of g.classes) {
        for (const m of c.methods) {
          rows.push({ className: c.name, methodName: m.name, isStatic: m.isStatic ?? null });
          if (rows.length >= limit) return { total: rows.length, rows };
        }
      }
    }
    return { total: rows.length, rows };
  });
}

/** 某个类的方法与构造函数清单（getDeclaredMethods/Constructors，P-02） */
export function listMethods(className: string): {
  methods: { name: string; signature: string; isStatic: boolean; isConstructor: boolean }[];
  error: string | null;
} {
  return performSync(() => {
    try {
      const C = Java.use(className);
      const out: { name: string; signature: string; isStatic: boolean; isConstructor: boolean }[] = [];
      const cls = C.class;
      const mods = (m: { getModifiers: () => number }) =>
        (m.getModifiers() & 0x0008) !== 0; // Modifier.STATIC
      const methods = cls.getDeclaredMethods() as unknown as Array<{
        getName: () => string;
        toGenericString: () => string;
        getModifiers: () => number;
      }>;
      for (const m of methods) {
        out.push({ name: m.getName(), signature: m.toGenericString(), isStatic: mods(m), isConstructor: false });
      }
      const ctors = cls.getDeclaredConstructors() as unknown as Array<{
        toGenericString: () => string;
        getModifiers: () => number;
      }>;
      for (const c of ctors) {
        out.push({
          name: "$init",
          signature: c.toGenericString(),
          isStatic: false,
          isConstructor: true,
        });
      }
      return { methods: out, error: null };
    } catch (e) {
      return { methods: [], error: String(e) };
    }
  });
}

// ---------------- 类加载器（加固壳，P-05） ----------------

export function listLoaders(): { index: number; toString: string }[] {
  return performSync(() => {
    const out: { index: number; toString: string }[] = [];
    Java.enumerateClassLoadersSync().forEach((l: unknown, i: number) => {
      out.push({ index: i, toString: String(l) });
    });
    return out;
  });
}

export function useLoader(index: number): { ok: boolean; current: string } {
  return performSync(() => {
    const loaders = Java.enumerateClassLoadersSync();
    if (index < 0 || index >= loaders.length) {
      return { ok: false, current: String(Java.classFactory.loader) };
    }
    (Java.classFactory as unknown as { loader: unknown }).loader = loaders[index];
    return { ok: true, current: String(Java.classFactory.loader) };
  });
}

// ---------------- 实例搜索（Java.choose） ----------------

export function chooseInstances(className: string, limit: number): Promise<{
  count: number;
  samples: string[];
}> {
  return new Promise((resolve) => {
    const samples: string[] = [];
    let count = 0;
    Java.choose(className, {
      onMatch(instance: unknown) {
        count++;
        if (samples.length < limit) {
          try {
            samples.push(String((instance as { toString(): string }).toString()));
          } catch {
            samples.push("<toString 抛错>");
          }
        }
      },
      onComplete() {
        resolve({ count, samples });
      },
    });
  });
}

// ---------------- 探针注册表（hooks） ----------------

export interface ProbeDecl {
  id: string;
  clazz: string;
  method: string; // "$init" 表示构造函数
  maxLen?: number;
  captureRet?: boolean;
  backtrace?: boolean;
  /** 可选条件表达式（js，this/arguments 可用，返回 boolean） */
  condition?: string;
}

interface ProbeState {
  decl: ProbeDecl;
  hits: number;
  errors: number;
  status: "waiting" | "active" | "error";
  lastError: string | null;
  installedAtLoader: string | null;
}

export const probes = new Map<string, ProbeState>();

function sendProbeEvent(payload: Record<string, unknown>): void {
  send(payload as unknown as { [key: string]: unknown });
}

function installOne(decl: ProbeDecl): void {
  const C = Java.use(decl.clazz);
  const methodRef = decl.method === "$init" ? C.$init : C[decl.method];
  if (!methodRef || typeof (methodRef as { overloads?: unknown }).overloads === "undefined") {
    throw new Error(`方法不存在或不可 hook：${decl.clazz}.${decl.method}（P-02）`);
  }
  const overloads = (methodRef as { overloads: Array<{ implementation: unknown; apply: (self: unknown, args: unknown[]) => unknown }> }).overloads;
  overloads.forEach((ov) => {
    ov.implementation = function (this: unknown, ...args: unknown[]) {
      const st = probes.get(decl.id);
      if (!st) {
        return ov.apply(this, args);
      }
      st.hits++;
      // 条件过滤（O-03 场景下配合使用；抛错按不过滤处理并计入 errors）
      if (decl.condition) {
        let pass = true;
        try {
          // eslint-disable-next-line @typescript-eslint/no-implied-eval
          const cond = new Function("this_", "arguments", `"use strict"; return (${decl.condition});`);
          pass = cond(this, args) as boolean;
        } catch (e) {
          st.errors++;
          st.lastError = `condition: ${String(e)}`;
          pass = true;
        }
        if (!pass) {
          return ov.apply(this, args);
        }
      }
      const maxLen = decl.maxLen ?? 128;
      const encArgs: EncValue[] = args.map((a) => encodeValue(a, maxLen));
      const payload: Record<string, unknown> = {
        t: "probe_hit",
        id: decl.id,
        clazz: decl.clazz,
        method: decl.method,
        ts: Date.now(),
        thread: Process.getCurrentThreadId(),
        args: encArgs,
      };
      try {
        const ret = ov.apply(this, args);
        if (decl.captureRet) {
          payload.ret = encodeValue(ret, maxLen);
        }
        sendProbeEvent(payload);
        return ret;
      } catch (e) {
        st.errors++;
        st.lastError = String(e);
        payload.args = encArgs;
        sendProbeEvent({ t: "probe_error", id: decl.id, phase: "invoke", error: String(e) });
        throw e;
      }
    };
  });
}

/**
 * 添加探针（声明式 → 运行时挂钩）。
 * 三态上报（P-06）：类未加载 → waiting（延迟重试，P-05/17.x 时序）；成功 → active；异常 → error。
 */
export function addProbes(decls: ProbeDecl[]): {
  results: { id: string; status: string; error: string | null }[];
} {
  const results: { id: string; status: string; error: string | null }[] = [];
  performSync(() => {
    for (const decl of decls) {
      const st: ProbeState = {
        decl,
        hits: 0,
        errors: 0,
        status: "waiting",
        lastError: null,
        installedAtLoader: null,
      };
      probes.set(decl.id, st);
      const attempt = (retry: number) => {
        try {
          installOne(decl);
          st.status = "active";
          st.installedAtLoader = String(Java.classFactory.loader);
          results.push({ id: decl.id, status: "active", error: null });
        } catch (e) {
          const msg = String(e);
          const classMissing = msg.includes("ClassNotFoundException") || msg.includes("java.lang.Class");
          if (classMissing && retry < 5) {
            // P-05/17.x 时序：类还没加载，延迟重试
            setTimeout(() => attempt(retry + 1), 1500 * (retry + 1));
            return;
          }
          st.status = "error";
          st.lastError = msg;
          results.push({ id: decl.id, status: "error", error: msg });
          sendProbeEvent({ t: "probe_error", id: decl.id, phase: "install", error: msg });
        }
      };
      attempt(0);
      if (st.status === "waiting" && !results.find((r) => r.id === decl.id)) {
        results.push({ id: decl.id, status: "waiting", error: "类未加载，延迟重试中（P-05）" });
      }
    }
  });
  return { results };
}

export function removeProbes(ids: string[]): { removed: number } {
  return performSync(() => {
    let removed = 0;
    for (const id of ids) {
      const st = probes.get(id);
      if (!st) continue;
      try {
        const C = Java.use(st.decl.clazz);
        const m = st.decl.method === "$init" ? C.$init : C[st.decl.method];
        (m as { overloads: Array<{ implementation: unknown }> }).overloads.forEach((ov) => {
          ov.implementation = null; // 恢复原始实现（可逆）
        });
        removed++;
      } catch {
        // 类已卸载等情况：视为已移除
        removed++;
      }
      probes.delete(id);
    }
    return { removed };
  });
}

export function probeStats(): {
  id: string;
  clazz: string;
  method: string;
  status: string;
  hits: number;
  errors: number;
  lastError: string | null;
}[] {
  const out: {
    id: string;
    clazz: string;
    method: string;
    status: string;
    hits: number;
    errors: number;
    lastError: string | null;
  }[] = [];
  probes.forEach((st, id) => {
    out.push({
      id,
      clazz: st.decl.clazz,
      method: st.decl.method,
      status: st.status,
      hits: st.hits,
      errors: st.errors,
      lastError: st.lastError,
    });
  });
  return out;
}
