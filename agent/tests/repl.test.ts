import { describe, expect, it, vi } from "vitest";

/** repl.ts 单测：批次⑩ 修复的跨调用持久化回归（strict direct eval 独立变量环境事故）。 */

vi.mock("frida-java-bridge", () => ({ default: {} }));

import { complete, evaluate } from "../src/repl";

describe("agent REPL evaluate", () => {
  it("回归：var 声明跨调用持久（此前 new Function('use strict'; eval) 的 var 出不了当次调用，跨调用必 ReferenceError）", () => {
    const key = `__lf_repl_persist_${Date.now()}`;
    const first = evaluate(`var ${key} = 41; ${key} + 1`);
    expect(first.value).toEqual({ k: "num", v: "42" });
    // 第二次调用必须能看到第一次声明的变量——REPL 持久化的核心承诺
    const second = evaluate(key);
    expect(second.value).toEqual({ k: "num", v: "41" });
  });

  it("回归：let/const 提升为 var 后重复声明不再抛 already declared（Luma 技巧在全局作用域下才成立）", () => {
    const key = `__lf_repl_let_${Date.now()}`;
    expect(evaluate(`let ${key} = 1`).value.k).not.toBe("err");
    const again = evaluate(`let ${key} = 2`);
    expect(again.value.k).not.toBe("err");
    expect(evaluate(key).value).toEqual({ k: "num", v: "2" });
  });

  it("求值错误返回 err 标签而不抛出（编码永不外泄异常）", () => {
    const r = evaluate("1 +");
    expect(r.value.k).toBe("err");
    const ref = evaluate("__lf_no_such_var_xyz");
    expect(ref.value.k).toBe("err");
  });

  it("对象/数组结果正常编码", () => {
    expect(evaluate("({a: 1})").value.k).toBe("obj");
    expect(evaluate("[1, 2]").value.k).toBe("arr");
  });
});

describe("agent REPL complete", () => {
  it("原型链补全：全局对象属性按前缀过滤", () => {
    const r = complete("JSON.str", 8);
    expect(r.base).toBe("JSON");
    expect(r.items.map((i) => i.name)).toContain("stringify");
  });

  it("无补全Token时返回空", () => {
    expect(complete("", 0).items).toEqual([]);
  });
});
