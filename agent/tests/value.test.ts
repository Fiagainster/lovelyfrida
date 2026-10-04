import { beforeEach, describe, expect, it, vi } from "vitest";

/** value.ts 单测：类型标签 / 截断 / 深度哨兵。frida-java-bridge 用 vi.mock 替身。 */

vi.mock("frida-java-bridge", () => ({
  default: {
    // Java.array("byte", arr) 的替身：直接返回数组（encodeValue 只读 length 与下标）
    array: (_type: string, arr: number[]) => arr,
  },
}));

import { encodeValue } from "../src/value";

describe("agent value 编码", () => {
  beforeEach(() => {
    vi.mocked;
  });

  it("基础类型：string/number/boolean/null/undefined 各归其标签", () => {
    expect(encodeValue("hello")).toEqual({ k: "str", v: "hello" });
    expect(encodeValue(42)).toEqual({ k: "num", v: "42" });
    expect(encodeValue(true)).toEqual({ k: "bool", v: "true" });
    expect(encodeValue(null)).toEqual({ k: "null", v: "null" });
    expect(encodeValue(undefined)).toEqual({ k: "undef", v: "undefined" });
  });

  it("超长字符串截断并附 …(+N) 与 total（O-02）", () => {
    const r = encodeValue("x".repeat(300), 128);
    expect(r.k).toBe("str");
    expect(r.v).toHaveLength(128 + "…(+172)".length);
    expect(r.v.endsWith("…(+172)")).toBe(true);
    expect(r.total).toBe(300);
  });

  it("不超长不写 total 字段", () => {
    expect(encodeValue("short", 128).total).toBeUndefined();
  });

  it("数组：深度递归编码、前 10 项 + 溢出哨兵", () => {
    const r = encodeValue([1, "a", true], 128);
    expect(r.k).toBe("arr");
    expect(r.v).toBe("[1, a, true]");
    const big = encodeValue(Array.from({ length: 15 }, (_, i) => i), 128);
    expect(big.v).toContain("…(+5)");
  });

  it("嵌套对象：depth 2 递归、超深收哨兵、键数截断 12", () => {
    const r = encodeValue({ a: { b: { c: { d: 1 } } } }, 128);
    expect(r.k).toBe("obj");
    expect(r.v).toContain("a=");
    // 深度耗尽的对象变成 Object.prototype.toString 形态
    expect(r.v).toContain("[object Object]");
    const many = encodeValue(
      Object.fromEntries(Array.from({ length: 20 }, (_, i) => [`k${i}`, i])),
      128,
    );
    expect(many.v).toContain("k11=");
    expect(many.v).not.toContain("k12=");
  });

  it("byte[] 语义对象（getClass().getName()==='[B'）→ hex 化 + 截断（O-01）", () => {
    const javaBytes = { getClass: () => ({ getName: () => "[B" }) };
    (javaBytes as unknown as Record<string, unknown>)[Symbol.toPrimitive as unknown as number] = undefined;
    // Java.array 替身直接回数组： encodeValue 遍历 0..min(n,maxLen)
    const payload = Object.assign(javaBytes, { length: 4, 0: 0x41, 1: 0x42, 2: 0x43, 3: 0x44 });
    const r = encodeValue(payload, 128);
    expect(r.k).toBe("bytes");
    expect(r.v).toBe("41424344");
    expect(r.total).toBe(4);
  });

  it("永不抛异常：怪异输入返回 err 标签（顶层 try/catch 契约）", () => {
    const evil: Record<string, unknown> = {};
    Object.defineProperty(evil, "getClass", {
      get() {
        throw new Error("boom");
      },
    });
    const r = encodeValue(evil, 128);
    expect(["err", "obj"]).toContain(r.k);
  });

  it("函数与 NativePointer 形态", () => {
    expect(encodeValue(() => 1).k).toBe("func");
    const ptrLike = { toString: () => "0x1234", constructor: { name: "NativePointer" } };
    expect(encodeValue(ptrLike)).toEqual({ k: "ptr", v: "0x1234" });
  });
});
