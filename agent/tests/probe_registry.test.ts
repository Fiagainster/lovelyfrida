import { describe, expect, it } from "vitest";

/** probe_registry.ts 单测：同方法位互斥（批次⑩ waiting 漏洞回归）+ 安装重试策略。 */

import {
  decideInstallError,
  findMethodSlotConflict,
  PROBE_MAX_RETRIES,
  probeRetryDelay,
  type ProbeDecl,
  type ProbeState,
} from "../src/probe_registry";

function decl(id: string, clazz = "com.x.Crypto", method = "hash"): ProbeDecl {
  return { id, clazz, method };
}

function state(id: string, status: ProbeState["status"], clazz = "com.x.Crypto", method = "hash"): ProbeState {
  return {
    decl: decl(id, clazz, method),
    hits: 0,
    errors: 0,
    status,
    lastError: null,
    installedAtLoader: null,
    condFn: null,
  };
}

describe("同方法位互斥 findMethodSlotConflict", () => {
  it("回归：waiting 态探针必须占用方法位（此前只查 active，重试竞态下后装者顶掉先装者）", () => {
    const probes = new Map([["p1", state("p1", "waiting")]]);
    expect(findMethodSlotConflict(probes, decl("p2"))).toBe("p1");
  });

  it("active 态冲突照旧拒绝（批次⑦联调事故场景）", () => {
    const probes = new Map([["p1", state("p1", "active")]]);
    expect(findMethodSlotConflict(probes, decl("p2"))).toBe("p1");
  });

  it("error 终态不占用方法位（可换探针重挂）", () => {
    const probes = new Map([["p1", state("p1", "error")]]);
    expect(findMethodSlotConflict(probes, decl("p2"))).toBeNull();
  });

  it("同 id 重挂不算冲突（UI 超时重试语义）", () => {
    const probes = new Map([["p1", state("p1", "active")]]);
    expect(findMethodSlotConflict(probes, decl("p1"))).toBeNull();
  });

  it("不同方法位/不同类互不冲突", () => {
    const probes = new Map([
      ["p1", state("p1", "active", "com.x.Crypto", "hash")],
      ["p2", state("p2", "waiting", "com.x.Crypto", "sign")],
      ["p3", state("p3", "active", "com.y.Other", "hash")],
    ]);
    expect(findMethodSlotConflict(probes, decl("p4", "com.x.Crypto", "verify"))).toBeNull();
  });
});

describe("安装失败决策 decideInstallError", () => {
  it("类未加载 → 延迟重试，间隔递增（P-05/17.x 时序）", () => {
    const msg = "java.lang.ClassNotFoundException: com.x.Crypto";
    for (let retry = 0; retry < PROBE_MAX_RETRIES; retry++) {
      const d = decideInstallError(msg, retry);
      expect(d.kind).toBe("retry");
      expect(d.kind === "retry" && d.delayMs).toBe(probeRetryDelay(retry));
    }
    expect(probeRetryDelay(0)).toBeLessThan(probeRetryDelay(PROBE_MAX_RETRIES - 1));
  });

  it("重试超上限 → error 终态（不无限重试）", () => {
    const d = decideInstallError("java.lang.ClassNotFoundException: X", PROBE_MAX_RETRIES);
    expect(d.kind).toBe("error");
  });

  it("非类缺失错误 → 立即 error（方法不存在等不重试）", () => {
    expect(decideInstallError("方法不存在或不可 hook：X.y（P-02）", 0).kind).toBe("error");
  });

  it("两类类缺失文案都能识别（heuristics 契约）", () => {
    expect(decideInstallError("ClassNotFoundException: X", 0).kind).toBe("retry");
    expect(decideInstallError("java.lang.Class not found: X", 0).kind).toBe("retry");
  });
});
