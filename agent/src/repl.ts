/**
 * REPL（文档09 §repl）：evaluate（let/const→var 提升，Luma 技巧）+ 原型链补全。
 */
import { encodeValue, EncValue } from "./value";

export interface ReplResult {
  value: EncValue;
  blob_b64: string | null;
}

export function evaluate(code: string): ReplResult {
  // let/const 提升（重复执行不报 already declared）
  const hoisted = code.replace(/^\s*(let|const)\s+/gm, "var ");
  let result: unknown;
  try {
    // 间接 eval：不继承调用方严格性（esbuild 产物整体严格模式），在全局作用域以
    // 非严格模式执行 → var 声明落 globalThis，跨调用持久。
    // 此前 new Function('"use strict"; …eval(…)') 是 strict direct eval，有独立变量
    // 环境，var 根本出不了当次调用，跨调用必 ReferenceError（批次⑩修复，有回归测试）。
    // eslint-disable-next-line @typescript-eslint/no-implied-eval
    result = (0, eval)(hoisted);
  } catch (e) {
    return { value: { k: "err", v: String(e) }, blob_b64: null };
  }
  // ArrayBuffer 结果 → blob 通道
  if (result instanceof ArrayBuffer) {
    const bytes = new Uint8Array(result);
    let bin = "";
    for (let i = 0; i < bytes.length; i++) bin += String.fromCharCode(bytes[i]);
    return {
      value: { k: "bytes", v: `<ArrayBuffer ${bytes.length}B>` },
      blob_b64: btoa(bin),
    };
  }
  return { value: encodeValue(result, 512, 3), blob_b64: null };
}

export function complete(code: string, cursor: number): { base: string; items: { name: string; callable: boolean }[] } {
  const upto = code.slice(0, cursor);
  const m = upto.match(/[A-Za-z0-9_$.]+$/);
  if (!m) return { base: "", items: [] };
  const token = m[0];
  const dot = token.lastIndexOf(".");
  const baseExpr = dot >= 0 ? token.slice(0, dot) : "this";
  const fragment = dot >= 0 ? token.slice(dot + 1) : token;
  try {
    // eslint-disable-next-line @typescript-eslint/no-implied-eval
    const fn = new Function(`"use strict"; return eval(${JSON.stringify(baseExpr)});`);
    const obj = fn.call(globalThis) as object;
    const seen = new Set<string>();
    const items: { name: string; callable: boolean }[] = [];
    let cur: object | null = obj as object;
    for (let depth = 0; depth < 6 && cur; depth++) {
      Object.getOwnPropertyNames(cur).forEach((name) => {
        if (seen.has(name) || !name.toLowerCase().startsWith(fragment.toLowerCase())) return;
        seen.add(name);
        let callable = false;
        try {
          callable = typeof (obj as Record<string, unknown>)[name] === "function";
        } catch {
          callable = false;
        }
        items.push({ name, callable });
      });
      cur = Object.getPrototypeOf(cur);
      if (items.length >= 256) break;
    }
    items.sort((a, b) => Number(a.callable) - Number(b.callable) || a.name.localeCompare(b.name));
    return { base: baseExpr, items: items.slice(0, 256) };
  } catch {
    return { base: baseExpr, items: [] };
  }
}
