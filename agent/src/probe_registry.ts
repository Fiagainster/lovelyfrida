/**
 * 探针注册表纯逻辑（文档09 §hooks）：互斥判定 + 安装重试策略。
 * 从 java.ts 抽出为无 frida 依赖的纯函数——这是出过两次同型真 bug 的代码
 * （批次⑦联调：同方法位顶掉；批次⑩：waiting 态互斥漏洞），必须可单测。
 */

export interface ProbeDecl {
  id: string;
  clazz: string;
  method: string; // "$init" 表示构造函数
  maxLen?: number;
  captureRet?: boolean;
  backtrace?: boolean;
  /** 可选条件表达式（js，变量 this_ = 调用对象、arguments = 参数数组，返回 truthy 才上报） */
  condition?: string;
}

export interface ProbeState {
  decl: ProbeDecl;
  hits: number;
  errors: number;
  status: "waiting" | "active" | "error";
  lastError: string | null;
  installedAtLoader: string | null;
  /** 安装期一次性编译的条件函数（编译失败 = 探针 error，不带病挂载） */
  condFn: ((this_: unknown, args: unknown[]) => unknown) | null;
}

/**
 * 同方法位互斥：`ov.implementation = …` 是替换语义，同一 clazz.method 只允许一个探针。
 * waiting 必须与 active 一并占用方法位——waiting 探针的延迟重试最终会 install，
 * 若放行第二个探针，两者的重试都会 install，后装者顶掉先装者，先装者状态照常置
 * active 但 hits 恒 0（批次⑩：此前只查 active，与批次⑦联调事故同型，只是换了时序）。
 * 同 id 重挂（UI 超时重试）不算冲突。
 */
export function findMethodSlotConflict(
  probes: ReadonlyMap<string, Pick<ProbeState, "decl" | "status">>,
  decl: ProbeDecl,
): string | null {
  for (const [id, s] of probes) {
    if (
      id !== decl.id &&
      (s.status === "active" || s.status === "waiting") &&
      s.decl.clazz === decl.clazz &&
      s.decl.method === decl.method
    ) {
      return id;
    }
  }
  return null;
}

export const PROBE_MAX_RETRIES = 5;

/** 安装失败延迟重试的递增间隔（P-05/17.x 时序：类加载晚于脚本就绪） */
export function probeRetryDelay(retry: number): number {
  return 1500 * (retry + 1);
}

/** 类未加载的错误识别（heuristics 对 frida-java-bridge 的两类报错文案） */
export function isClassMissingError(msg: string): boolean {
  return msg.includes("ClassNotFoundException") || msg.includes("java.lang.Class");
}

export type InstallDecision = { kind: "retry"; delayMs: number } | { kind: "error" };

/** 安装失败后的决策：类未加载且未超重试上限 → 延迟重试；否则 → error 终态 */
export function decideInstallError(msg: string, retry: number): InstallDecision {
  if (retry < PROBE_MAX_RETRIES && isClassMissingError(msg)) {
    return { kind: "retry", delayMs: probeRetryDelay(retry) };
  }
  return { kind: "error" };
}
