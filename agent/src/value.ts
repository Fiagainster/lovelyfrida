/**
 * 值编码（文档09 §value）：把 hook 参数/返回值/REPL 结果编码为带类型标签的 JSON。
 * byte[] 一律 hex 化（O-01）；超长截断并附 …(+N)（O-02）；深度 2 + 深度哨兵。
 */
import Java from "frida-java-bridge";

export interface EncValue {
  k:
    | "str"
    | "num"
    | "bool"
    | "bytes"
    | "obj"
    | "arr"
    | "null"
    | "undef"
    | "ptr"
    | "err"
    | "func"
    | "unknown";
  v: string;
  /** 截断前总长（仅截断时出现） */
  total?: number;
}

function trunc(s: string, maxLen: number): { v: string; total?: number } {
  if (s.length <= maxLen) return { v: s };
  return { v: s.slice(0, maxLen) + "…(+" + (s.length - maxLen) + ")", total: s.length };
}

export function encodeValue(v: unknown, maxLen = 128, depth = 2): EncValue {
  try {
    if (v === null || v === undefined) {
      return { k: v === null ? "null" : "undef", v: String(v) };
    }
    const t = typeof v;
    if (t === "string") {
      return { k: "str", ...trunc(v as string, maxLen) };
    }
    if (t === "number") return { k: "num", v: String(v) };
    if (t === "boolean") return { k: "bool", v: String(v) };
    if (t === "function") {
      return { k: "func", ...trunc(String(v), maxLen) };
    }
    // NativePointer / NativePointer-like
    if (
      typeof (v as { toString?: unknown }).toString === "function" &&
      (v as { constructor?: { name?: string } }).constructor?.name === "NativePointer"
    ) {
      return { k: "ptr", v: (v as { toString(): string }).toString() };
    }
    // 字节数组（Java byte[] 包装：getClass().getName() === "[B"）
    const jv = v as { getClass?: () => { getName: () => string } };
    if (typeof jv.getClass === "function") {
      let clsName = "";
      try {
        clsName = jv.getClass().getName();
      } catch {
        clsName = "";
      }
      if (clsName === "[B") {
        try {
          const bytes = Java.array("byte", v as unknown as number[]);
          let hex = "";
          const n = bytes.length;
          for (let i = 0; i < Math.min(n, maxLen); i++) {
            const b = bytes[i] & 0xff;
            hex += (b < 16 ? "0" : "") + b.toString(16);
          }
          if (n > maxLen) hex += "…(+" + (n - maxLen) + "B)";
          return { k: "bytes", v: hex, total: n };
        } catch {
          /* fallthrough */
        }
      }
      // 其他 Java 对象：toString + 类名
      try {
        const s = (v as { toString(): string }).toString();
        return { k: "obj", ...trunc(`[${clsName}] ${s}`, maxLen) };
      } catch {
        return { k: "obj", v: `[${clsName}] <toString 抛错>` };
      }
    }
    // JS 数组
    if (Array.isArray(v)) {
      if (depth <= 0) return { k: "arr", v: `Array(${v.length}) …` };
      const items = v.slice(0, 10).map((x) => encodeValue(x, maxLen, depth - 1));
      const more = v.length > 10 ? [`…(+${v.length - 10})`] : [];
      return {
        k: "arr",
        ...trunc(
          "[" + items.map((x) => x.v).concat(more).join(", ") + "]",
          maxLen * 4,
        ),
      };
    }
    // 普通对象（含 JS plain object / Map 等）
    if (depth <= 0) return { k: "obj", v: Object.prototype.toString.call(v) };
    if (typeof (v as { toJSON?: unknown }).toJSON === "function") {
      try {
        return encodeValue((v as { toJSON(): unknown }).toJSON(), maxLen, depth);
      } catch {
        /* fallthrough */
      }
    }
    const keys = Object.keys(v as object).slice(0, 12);
    const parts = keys.map((k) => `${k}=${encodeValue((v as Record<string, unknown>)[k], maxLen, depth - 1).v}`);
    return { k: "obj", ...trunc("{" + parts.join(", ") + "}", maxLen * 2) };
  } catch (e) {
    return { k: "err", v: `<编码失败: ${String(e)}>` };
  }
}
