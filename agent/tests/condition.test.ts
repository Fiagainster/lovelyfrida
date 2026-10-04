import { describe, expect, it } from "vitest";

/** condition.ts 单测：条件表达式编译（O-03 曾在此全灭——严格模式 arguments 形参）。 */

import { compileCondition } from "../src/condition";

describe("agent 条件表达式编译", () => {
  it("合法表达式编译并求值：arguments/this_ 变量可用（UI 承诺的变量名契约）", () => {
    const fn = compileCondition("arguments.length > 0 && String(arguments[0]) === 'X'");
    expect(fn({}, ["X"])).toBeTruthy();
    expect(fn({}, ["Y"])).toBeFalsy();
  });

  it("回归：不带 use strict —— arguments 作形参名不再 SyntaxError（此前条件过滤从未生效的根因）", () => {
    // 若此用例失败 = 有人把 "use strict" 加回了编译模板
    expect(() => compileCondition("arguments[0] !== undefined")).not.toThrow();
    const fn = compileCondition("arguments.length === 2");
    expect(fn(null, [1, 2])).toBe(true);
  });

  it("语法错误在编译期抛出（安装期即探针 error，不带病挂载）", () => {
    expect(() => compileCondition("this is not js;;;")).toThrow(SyntaxError);
    expect(() => compileCondition("return")).toThrow();
  });

  it("truthy 语义：非布尔返回值按 truthy 过滤（与 hook 内 Boolean() 包装一致）", () => {
    const fn = compileCondition("arguments[0]");
    expect(fn(null, ["non-empty"])).toBeTruthy();
    expect(fn(null, [""])).toBeFalsy();
    expect(fn(null, [0])).toBeFalsy();
  });

  it("this_ 变量指向调用对象", () => {
    const fn = compileCondition("this_ && this_.tag === 'ok'");
    expect(fn.call(null, { tag: "ok" }, [])).toBeTruthy();
    expect(fn.call(null, { tag: "no" }, [])).toBeFalsy();
  });
});
