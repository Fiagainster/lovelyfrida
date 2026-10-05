/**
 * 静态符号清单解析（D6/B3 落地半边）：把 jadx 等静态工具导出的目标方法清单
 * 归一化为探针候选。独立纯函数模块——零依赖、可 vitest 直测。
 *
 * 接受两种形态（可混排，逐行容错）：
 *   1. 点号行：`com.notevault.app.utils.AESUtil.hashPassword`（最后一个 . 前是类名）
 *   2. JSON 数组：`[{"clazz": "...", "method": "..."}]`（多余字段忽略）
 * 输出去重（clazz+method）、剔空、上限 200（防粘贴失控）。
 */

export interface StaticSymbol {
  clazz: string;
  method: string;
}

export const SYMBOLS_CAP = 200;

export function parseSymbols(text: string): StaticSymbol[] {
  const out: StaticSymbol[] = [];
  const seen = new Set<string>();
  const push = (clazz: string, method: string) => {
    const c = clazz.trim();
    const m = method.trim();
    if (!c || !m || out.length >= SYMBOLS_CAP) return;
    const key = `${c}#${m}`;
    if (seen.has(key)) return;
    seen.add(key);
    out.push({ clazz: c, method: m });
  };

  const trimmed = text.trim();
  if (!trimmed) return out;

  // JSON 数组形态（整段以 [ 开头才尝试；失败按行容错继续）
  if (trimmed.startsWith("[")) {
    try {
      const arr = JSON.parse(trimmed) as unknown;
      if (Array.isArray(arr)) {
        for (const item of arr) {
          if (item && typeof item === "object") {
            const o = item as Record<string, unknown>;
            if (typeof o.clazz === "string" && typeof o.method === "string") {
              push(o.clazz, o.method);
            }
          }
        }
        return out;
      }
    } catch {
      /* JSON 残缺：按行继续容错 */
    }
  }

  // 点号行形态
  for (const raw of trimmed.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line || line.startsWith("#") || line.startsWith("//")) continue;
    // JSON 碎片行（残缺数组被截断的残余）不是点号形态：跳过防垃圾候选
    if (line.includes('"') || line.startsWith("[") || line.startsWith("{")) continue;
    const dot = line.lastIndexOf(".");
    if (dot <= 0 || dot === line.length - 1) continue;
    push(line.slice(0, dot), line.slice(dot + 1));
  }
  return out;
}
