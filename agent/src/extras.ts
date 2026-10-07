/**
 * 能力包 A/C（文档09 §native 补强）：
 * - dumpDex：内存中搜 dex 魔数并回传（脱壳辅助，只读观测）
 * - dlopen 监控：hook dlopen/android_dlopen_ext，事件上报（加固 so 加载观测）
 * - RegisterNatives 捕获：JNI 动态注册观测（native 方法定位）
 * - SSL 缓冲捕获探针：SSL_read/SSL_write 的缓冲区内容观测（r0capture 风格，只观测）
 * - 实例调用：Java.choose + 按 hashCode 定向调用实例方法
 */
import Java from "frida-java-bridge";
import { emitEvent } from "./batch";

// ---------------- dlopen 监控 ----------------


/** 跨 frida 版本的导出解析（17.x 移除了静态 getExportByName；getGlobalExportByName 为新 API）。
 *  modName 给定时优先在该模块内解析（避免同名符号被全局解析到错误模块），失败再全局兜底。 */
export function resolveExport(modName: string | null, name: string): NativePointer | null {
  const M = Module as unknown as Record<string, unknown>;
  // 1) 模块内实例方法（17.x 推荐；find 变体不抛错优先）
  if (modName !== null) {
    try {
      const m = (Process as unknown as { findModuleByName: (n: string) => unknown }).findModuleByName(modName);
      if (m) {
        const inst = m as unknown as Record<string, unknown>;
        for (const meth of ["findExportByName", "getExportByName"]) {
          if (typeof inst[meth] === "function") {
            try {
              const p = (inst[meth] as (n: string) => NativePointer)(name);
              if (p && !p.isNull()) return p;
            } catch { /* next */ }
          }
        }
      }
    } catch { /* continue */ }
  }
  // 2) 旧静态 API（frida ≤16；17.x 已移除，需 typeof 探测）
  if (typeof M.findExportByName === "function") {
    try {
      const p = (M.findExportByName as (m: string | null, n: string) => NativePointer | null)(modName, name);
      if (p && !p.isNull()) return p;
    } catch { /* continue */ }
  }
  // 3) 17.x 全局搜索兜底（模块内没有/未给模块名时）
  if (typeof M.getGlobalExportByName === "function") {
    try {
      const p = (M.getGlobalExportByName as (n: string) => NativePointer)(name);
      if (p && !p.isNull()) return p;
    } catch { /* not found */ }
  }
  return null;
}

let dlopenWatchActive = false;

export function watchDlopen(): { ok: boolean; hooks: number; error: string | null } {
  if (dlopenWatchActive) return { ok: true, hooks: 2, error: null };
  let hooks = 0;
  const errs: string[] = [];
  for (const name of ["dlopen", "android_dlopen_ext"]) {
    try {
      const addr = resolveExport("libdl.so", name) ?? resolveExport(null, name);
      if (!addr) {
        errs.push(`${name}: 符号未找到`);
        continue;
      }
      Interceptor.attach(addr, {
        onEnter(args) {
          const path = args[0].readCString() ?? "";
          emitEvent({ t: "dlopen", path, thread: Process.getCurrentThreadId() });
        },
      });
      hooks++;
    } catch (e) {
      errs.push(`${name}: ${String(e)}`);
    }
  }
  if (hooks === 0) {
    return { ok: false, hooks: 0, error: errs.join("; ") };
  }
  dlopenWatchActive = true;
  return { ok: true, hooks, error: errs.length ? errs.join("; ") : null };
}

// ---------------- RegisterNatives 捕获 ----------------

let regNativesActive = false;

/** libart.so 的 RegisterNatives 符号在各版本上有差异，逐个候选尝试 */
export function watchRegisterNatives(): { ok: boolean; symbol: string | null; error: string | null } {
  if (regNativesActive) return { ok: true, symbol: "already", error: null };
  const candidates = [
    "_ZN3art3JNI15RegisterNativesEP7_JNIEnvP7_jclassPK15JNINativeMethodi",
    "_ZN3art9JavaVMExt15RegisterNativesEP7_JNIEnvP7_jclassPK15JNINativeMethodi",
  ];
  for (const sym of candidates) {
    const addr = resolveExport("libart.so", sym);
    if (!addr) continue;
    try {
      Interceptor.attach(addr, {
        onEnter(args) {
          // args: JNIEnv*, jclass, JNINativeMethod* methods, jint count
          const env = args[0];
          const clazz = args[1];
          const methods = args[2];
          const count = args[3].toInt32();
          // jclass → 类名（通过 JNI GetStringPtr 层太深，改用 Java 触发侧补齐；
          // 此处先记录方法指针数组，类名由 onLeave 侧 Java.current class 补）
          let className = "";
          try {
            const jenv = Java.vm.getEnv();
            const clsNamePtr = jenv.getClassName(clazz as unknown as object);
            className = String(clsNamePtr);
          } catch {
            className = "";
          }
          const natives: { name: string; sig: string; fnPtr: string }[] = [];
          const NativesCap = 64;
          // JNINativeMethod = { const char* name; const char* signature; void* fnPtr; } 每项 3 指针
          for (let i = 0; i < Math.min(count, NativesCap); i++) {
            try {
              const namePtr = methods.add(i * Process.pointerSize * 3).readPointer();
              const sigPtr = methods.add(i * Process.pointerSize * 3 + Process.pointerSize).readPointer();
              const fnPtr = methods.add(i * Process.pointerSize * 3 + Process.pointerSize * 2).readPointer();
              natives.push({
                name: namePtr.readCString() ?? "",
                sig: sigPtr.readCString() ?? "",
                fnPtr: fnPtr.toString(),
              });
            } catch {
              break;
            }
          }
          // 截断必须留痕：count > 上限时静默丢弃会让人误以为注册表就是这 64 条
          emitEvent({
            t: "register_natives",
            class: className,
            count,
            natives,
            truncated: count > NativesCap,
            env: String(env),
          });
        },
      });
      regNativesActive = true;
      return { ok: true, symbol: sym, error: null };
    } catch (e) {
      /* 尝试下一个 */
    }
  }
  return { ok: false, symbol: null, error: "libart.so 中未找到 RegisterNatives 符号（版本差异，P-08 邻域）" };
}

// ---------------- 内存 dump dex（脱壳辅助） ----------------

export function dumpDex(maxDex: number): {
  found: { base: string; size: number; header: string }[];
  dumped: { base: string; size: number }[];
  skipped_ranges: number;
  error: string | null;
} {
  const found: { base: string; size: number; header: string }[] = [];
  const dumped: { base: string; size: number }[] = [];
  let skipped_ranges = 0;
  const DEX_MAGIC = "64 65 78 0a 30 33 ?? 00"; // dex\n03?\0（035/037/038/039）
  const ranges = Process.enumerateRanges("r--").concat(Process.enumerateRanges("rw-"));
  for (const r of ranges) {
    if (found.length >= maxDex) break;
    if (r.size < 112 || r.size > 100 * 1024 * 1024) {
      // >100MB 的堆区里也可能有 dex：跳过必须计数留痕，不能无声消失
      if (r.size > 100 * 1024 * 1024) skipped_ranges++;
      continue;
    }
    try {
      // 在区段内扫描 dex 魔数（头部不一定在映射起始位置）
      const matches = Memory.scanSync(r.base, r.size, DEX_MAGIC);
      for (const m of matches) {
        if (found.length >= maxDex) break;
        try {
          const u8 = new Uint8Array(m.address.readByteArray(8) ?? new Uint8Array(0));
          const ver = String.fromCharCode(u8[4], u8[5], u8[6]);
          // header 偏移 32 处是 file_size（u32 LE），做合法性校验
          const fsizeBuf = m.address.add(32).readByteArray(4);
          const fsize = fsizeBuf ? new Uint8Array(fsizeBuf) : null;
          const fileSize = fsize ? fsize[0] | (fsize[1] << 8) | (fsize[2] << 16) | (fsize[3] << 24) : 0;
          if (fileSize < 112 || fileSize > 80 * 1024 * 1024) continue;
          const baseStr = m.address.toString();
          found.push({ base: baseStr, size: fileSize, header: "dex\n0" + ver });
          // 单条 send 上限 60MB：超限 dex 截断落盘会得到损坏文件——必须带标记，
          // 让调用方知道「dumped_size < file_size = 证据不完整」而不是当成完整 dex
          const dumpedBytes = Math.min(fileSize, 60 * 1024 * 1024);
          const buf = m.address.readByteArray(dumpedBytes);
          if (buf) {
            send(
              {
                t: "dex_dump",
                base: baseStr,
                size: fileSize,
                dumped_size: dumpedBytes,
                truncated: dumpedBytes < fileSize,
              },
              buf as ArrayBuffer,
            );
            dumped.push({ base: baseStr, size: fileSize });
          }
        } catch {
          continue;
        }
      }
    } catch {
      continue;
    }
  }
  return { found, dumped, skipped_ranges, error: null };
}

// ---------------- SSL 缓冲捕获探针（r0capture 风格，只观测） ----------------

interface SslWatchState {
  id: string;
  hits: number;
}

const sslWatches = new Map<string, SslWatchState>();
// 每个导出地址只挂一次 listener：此前每个 watch 叠挂一层，N 次 watch = 每次读写 N 条重复事件
const sslListeners = new Map<string, InvocationListener>();
let sslMaxBuf = 512;
const SSL_FNS = ["SSL_read", "SSL_write"] as const;

export function watchSsl(
  id: string,
  maxBuf: number,
): { ok: boolean; hooks: string[]; error: string | null } {
  if (sslWatches.has(id)) return { ok: true, hooks: ["already"], error: null };
  sslMaxBuf = Math.max(sslMaxBuf, maxBuf);
  sslWatches.set(id, { id, hits: 0 });
  const hooked: string[] = [];
  let err: string | null = null;
  // Android 的 libssl 可能叫 libssl.so；conscrypt 场景走 Java 层 SSLOutputStream——v1 先 native
  for (const fn of SSL_FNS) {
    if (sslListeners.has(fn)) {
      hooked.push(fn); // 已有共享 hook：本 watch 直接复用
      continue;
    }
    try {
      const addr = resolveExport("libssl.so", fn);
      if (!addr) {
        err = `${fn}: libssl.so 符号未找到（部分模拟器 ROM 裁剪）`;
        continue;
      }
      const listener = Interceptor.attach(addr, {
        onEnter(args) {
          this._buf = args[1];
          this._len = args[2].toInt32();
          this._fn = fn;
        },
        onLeave(retval) {
          const len = this._fn === "SSL_write" ? this._len : retval.toInt32();
          if (len <= 0) return;
          const n = Math.min(len, sslMaxBuf);
          let buf: ArrayBuffer;
          try {
            buf = this._buf.readByteArray(n) as ArrayBuffer;
          } catch {
            return; /* 缓冲不可读跳过 */
          }
          const preview = (() => {
            try {
              return this._buf.readUtf8String(Math.min(n, 256));
            } catch {
              return null;
            }
          })();
          if (sslWatches.size === 0) return; /* listener 残留防御 */
          // 每 watch 计数照旧；事件只发一条（watches 数组承载归属）——
          // 此前 N 个 watch 发 N 条同 buffer 事件，高频 SSL_read 下序列化与 IPC 全是 N 倍放大（批次⑫）
          const watches: string[] = [];
          sslWatches.forEach((st, wid) => {
            st.hits++;
            watches.push(wid);
          });
          send(
            {
              t: "ssl_data",
              watches,
              fn: this._fn,
              len,
              thread: Process.getCurrentThreadId(),
              preview,
            },
            buf,
          );
        },
      });
      sslListeners.set(fn, listener);
      hooked.push(fn);
    } catch (e) {
      err = `${fn}: ${String(e)}`;
    }
  }
  if (hooked.length === 0 && sslListeners.size === 0) {
    sslWatches.delete(id);
    return { ok: false, hooks: [], error: err };
  }
  return { ok: true, hooks: hooked, error: err };
}

/** 卸载 SSL 监视；最后一个 watch 移除时真 detach（幽灵 hook 常驻高频函数是性能事故） */
export function unwatchSsl(id: string): { removed: number; detached: boolean } {
  const removed = sslWatches.delete(id) ? 1 : 0;
  let detached = false;
  if (sslWatches.size === 0 && sslListeners.size > 0) {
    sslListeners.forEach((l) => {
      try {
        l.detach();
      } catch {
        /* 目标已卸载 */
      }
    });
    sslListeners.clear();
    detached = true;
  }
  return { removed, detached };
}

export function sslStats(): { id: string; hits: number }[] {
  const out: { id: string; hits: number }[] = [];
  sslWatches.forEach((st, id) => out.push({ id, hits: st.hits }));
  return out;
}

// ---------------- 实例检查器 + 定向调用 ----------------

export function chooseInstancesDetailed(
  className: string,
  limit: number,
): { count: number; instances: { hashCode: number; toString: string }[] } {
  let count = 0;
  const instances: { hashCode: number; toString: string }[] = [];
  Java.choose(className, {
    onMatch(instance: unknown) {
      count++;
      if (instances.length < limit) {
        let toString = "";
        let hashCode = 0;
        try {
          toString = String((instance as { toString(): string }).toString());
        } catch {
          toString = "<toString 抛错>";
        }
        try {
          hashCode = (instance as { hashCode(): number }).hashCode();
        } catch {
          hashCode = 0;
        }
        instances.push({ hashCode, toString });
      }
    },
    onComplete() {
      /* done */
    },
  });
  return { count, instances };
}

/** 在实例上调用方法：重新 choose 并按 hashCode 定向（frida-java-bridge 的句柄不可跨 RPC 持久化） */
export function invokeOnInstance(
  className: string,
  hashCode: number,
  methodName: string,
  argsStr: string[],
): { ok: boolean; result: string; error: string | null } {
  try {
    let result = "<未找到实例>";
    let ok = false;
    Java.choose(className, {
      onMatch(instance: unknown) {
        if (ok) return;
        let hc = 0;
        try {
          hc = (instance as { hashCode(): number }).hashCode();
        } catch {
          return;
        }
        if (hc !== hashCode) return;
        const C = Java.use(className);
        const wrapped = Java.cast(instance as never, C as never) as unknown as Record<string, { apply: (self: unknown, args: unknown[]) => unknown }>;
        const method = wrapped[methodName];
        const jargs = argsStr.map((a) => {
          try {
            return Java.use("java.lang.String").$new(a);
          } catch {
            return a;
          }
        });
        const ret = method.apply(wrapped, jargs);
        result = String(ret);
        ok = true;
      },
      onComplete() {
        /* done */
      },
    });
    return { ok, result, error: null };
  } catch (e) {
    return { ok: false, result: "", error: String(e) };
  }
}
