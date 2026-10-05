import { describe, expect, it } from "vitest";

/** 静态符号解析单测（D6/B3）：点号行 / JSON 数组 / 混排容错 / 去重 / 上限。 */

import { parseSymbols, SYMBOLS_CAP } from "./symbols";

describe("parseSymbols", () => {
  it("点号行：最后一个 . 前是类名（内部类与包名不误切）", () => {
    const r = parseSymbols(
      "com.notevault.app.utils.AESUtil.hashPassword\njava.lang.Math.abs",
    );
    expect(r).toEqual([
      { clazz: "com.notevault.app.utils.AESUtil", method: "hashPassword" },
      { clazz: "java.lang.Math", method: "abs" },
    ]);
  });

  it("JSON 数组形态：多余字段忽略、非字符串字段剔除", () => {
    const r = parseSymbols(
      `[{"clazz":"com.a.B","method":"m","note":"jadx"},{"clazz":"com.a.B","method":123},{"bad":1}]`,
    );
    expect(r).toEqual([{ clazz: "com.a.B", method: "m" }]);
  });

  it("混排与容错：注释行跳过、空行跳过、残缺 JSON 按行继续", () => {
    const r = parseSymbols(
      "# 注释\n// 也是注释\n\ncom.a.B.m1\n[{\"clazz\":\"com.a.B\",\"method\":\"m2\"}\ncom.a.B.m3",
    );
    // JSON 碎片行（残缺数组残余）不产生垃圾候选：m2 被丢弃
    expect(r.map((x) => x.method)).toEqual(["m1", "m3"]);
  });

  it("去重（clazz+method）与剔空", () => {
    const r = parseSymbols("com.a.B.m\ncom.a.B.m\ncom.a.B.\n.m\n  com.a.B.m  ");
    expect(r).toEqual([{ clazz: "com.a.B", method: "m" }]);
  });

  it(`上限 ${SYMBOLS_CAP}：防粘贴失控`, () => {
    const many = Array.from({ length: 500 }, (_, i) => `com.a.B.m${i}`).join("\n");
    expect(parseSymbols(many)).toHaveLength(SYMBOLS_CAP);
  });

  it("空输入返回空数组", () => {
    expect(parseSymbols("")).toEqual([]);
    expect(parseSymbols("   \n  ")).toEqual([]);
  });
});
