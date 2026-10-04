/**
 * 探针条件表达式编译（O-03）。独立成模块：不依赖任何 frida 全局，
 * 使其可以在 Node（vitest）里单测——它是 agent 层出过真 bug 的地方
 * （此前 "use strict" 内联 + arguments 形参 = SyntaxError，条件过滤从未生效）。
 */

export type CondFn = (this_: unknown, args: unknown[]) => unknown;

/**
 * 把用户条件表达式编译为可调用函数（安装期一次性编译，失败直接探针 error）。
 * 变量契约（UI 提示同此）：this_ = 调用对象、arguments = 参数数组，返回 truthy 才上报。
 * 刻意不带 "use strict"：严格模式下 arguments 不能作形参名（SyntaxError），
 * 而 arguments 恰恰是文档承诺给用户的变量名——宽松模式函数体是唯一正确解。
 */
export function compileCondition(expr: string): CondFn {
  // eslint-disable-next-line @typescript-eslint/no-implied-eval
  return new Function("this_", "arguments", `return (${expr});`) as CondFn;
}
