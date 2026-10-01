/**
 * 能力包 A/C（文档09 §native 补强）：
 * - dumpDex：内存中搜 dex 魔数并回传（脱壳辅助，只读观测）
 * - dlopen 监控：hook dlopen/android_dlopen_ext，事件上报（加固 so 加载观测）
 * - RegisterNatives 捕获：JNI 动态注册观测（native 方法定位）
 * - SSL 缓冲捕获探针：SSL_read/SSL_write 的缓冲区内容观测（r0capture 风格，只观测）
 * - 实例调用：Java.choose + 按 hashCode 定向调用实例方法
 */
import Java from "frida-java-bridge";

// ---------------- dlopen 监控 ----------------

let dlopenWatchActive = false;

export function watchDlopen(): { ok: boolean; hooks: number; error: string | null } {
  if (dlopenWatchActive) return { ok: true, hooks: 2, error: null };
  let hooks = 0;
  const errs: string[] = [];
  for (const name of ["dlopen", "android_dlopen_ext"]) {
    try {
      const M = Module as unknown as { getExportByName: (m: string | null, n: string) => NativePointer };
      const addr = M.getExportByName("libdl.so", name) ?? M.getExportByName(null, name);
      Interceptor.attach(addr, {
        onEnter(args) {
          const path = args[0].readCString() ?? "";
          send({ t: "dlopen", path, thread: Process.getCurrentThreadId() });
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
    const addr = (Module as unknown as { findExportByName: (m: string, n: string) => NativePointer | null }).findExportByName("libart.so", sym);
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
          // JNINativeMethod = { const char* name; const char* signature; void* fnPtr; } 每项 3 指针
          for (let i = 0; i < Math.min(count, 64); i++) {
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
          send({ t: "register_natives", class: className, count, natives, env: String(env) });
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
  error: string | null;
} {
  const found: { base: string; size: number; header: string }[] = [];
  const dumped: { base: string; size: number }[] = [];
  const ranges = Process.enumerateRanges("r--").concat(Process.enumerateRanges("rw-"));
  for (const r of ranges) {
    if (found.length >= maxDex) break;
    if (r.size < 112 || r.size > 100 * 1024 * 1024) continue;
    try {
      const magic = r.base.readByteArray(8);
      if (!magic) continue;
      const u8 = new Uint8Array(magic);
      // "dex\n035\0" / 037 / 038 / 039
      if (u8[0] === 0x64 && u8[1] === 0x65 && u8[2] === 0x78 && u8[3] === 0x0a) {
        const ver = String.fromCharCode(u8[4], u8[5], u8[6]);
        const base = r.base.toString();
        found.push({ base, size: r.size, header: "dex\n0" + ver });
        // 回传完整内存块（data 通道），宿主写文件
        try {
          const buf = r.base.readByteArray(Math.min(r.size, 60 * 1024 * 1024));
          if (buf) {
            send({ t: "dex_dump", base, size: r.size }, buf as ArrayBuffer);
            dumped.push({ base, size: r.size });
          }
        } catch (e) {
          /* 读失败的区域跳过 */
        }
      }
    } catch {
      continue;
    }
  }
  return { found, dumped, error: null };
}

// ---------------- SSL 缓冲捕获探针（r0capture 风格，只观测） ----------------

interface SslWatchState {
  id: string;
  hits: number;
}

const sslWatches = new Map<string, SslWatchState>();

export function watchSsl(
  id: string,
  maxBuf: number,
): { ok: boolean; hooks: string[]; error: string | null } {
  if (sslWatches.has(id)) return { ok: true, hooks: ["already"], error: null };
  const hooked: string[] = [];
  let err: string | null = null;
  const st: SslWatchState = { id, hits: 0 };
  sslWatches.set(id, st);
  // Android 的 libssl 可能叫 libssl.so；conscrypt 场景走 Java 层 SSLOutputStream——v1 先 native
  for (const fn of ["SSL_read", "SSL_write"]) {
    try {
      const addr = (Module as unknown as { getExportByName: (m: string, n: string) => NativePointer }).getExportByName("libssl.so", fn);
      Interceptor.attach(addr, {
        onEnter(args) {
          this._buf = args[1];
          this._len = args[2].toInt32();
          this._fn = fn;
        },
        onLeave(retval) {
          const len = this._fn === "SSL_write" ? this._len : retval.toInt32();
          if (len <= 0) return;
          st.hits++;
          const n = Math.min(len, maxBuf);
          try {
            const buf = this._buf.readByteArray(n);
            send(
              {
                t: "ssl_data",
                watch: id,
                fn: this._fn,
                len,
                thread: Process.getCurrentThreadId(),
                preview: (() => {
                  try {
                    return this._buf.readUtf8String(Math.min(n, 256));
                  } catch {
                    return null;
                  }
                })(),
              },
              buf as ArrayBuffer,
            );
          } catch {
            /* 缓冲不可读跳过 */
          }
        },
      });
      hooked.push(fn);
    } catch (e) {
      err = `${fn}: ${String(e)}`;
    }
  }
  if (hooked.length === 0) return { ok: false, hooks: [], error: err };
  return { ok: true, hooks: hooked, error: err };
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
