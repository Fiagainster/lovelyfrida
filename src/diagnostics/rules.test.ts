import { describe, expect, it } from "vitest";
import { evaluateRules, type DiagContext } from "@/diagnostics/rules";
import type { CheckResult, ProbeStat } from "@/api";

/** 诊断规则引擎单测（A3）：docs/05 规则表 → 谓词 → 卡片 */

function chk(id: string, status: CheckResult["status"]): CheckResult {
  return {
    id,
    name: id,
    status,
    evidence: [],
    fix: null,
    rule: null,
    command: null,
    duration_ms: 0,
  };
}

function probe(p: Partial<ProbeStat>): ProbeStat {
  return {
    id: "p1",
    clazz: "com.example.Crypto",
    method: "hash",
    status: "active",
    hits: 0,
    errors: 0,
    lastError: null,
    ...p,
  };
}

function ctx(over: Partial<DiagContext>): DiagContext {
  return {
    doctor: null,
    session: null,
    serverStatus: null,
    probes: [],
    injection: null,
    signals: [],
    activeFindings: [],
    zeroHitSince: new Map(),
    waitingSince: new Map(),
    bruteSelfTestFailed: false,
    bruteEngineGenerateC: false,
    now: 1_000_000,
    ...over,
  };
}

function fired(ctx_: DiagContext): string[] {
  return evaluateRules(ctx_).map((c) => c.ruleId);
}

describe("evaluateRules", () => {
  it("无异常状态时不出卡", () => {
    expect(fired(ctx({}))).toEqual([]);
  });

  it("E-01：CHK-01 失败出阻断卡", () => {
    const cards = fired(ctx({ doctor: { checks: [chk("CHK-01", "fail")], overall: "fail", mode: "quick", adb_path: "", adb_source: "", device_serial: null, duration_ms: 0 } }));
    expect(cards).toContain("E-01");
  });

  it("E-02/E-03：CHK-03 失败 + 信号文本分流", () => {
    const base = { checks: [chk("CHK-03", "fail")], overall: "fail" as const, mode: "quick" as const, adb_path: "", adb_source: "", device_serial: null, duration_ms: 0 };
    expect(fired(ctx({ doctor: { ...base, checks: [chk("CHK-03", "fail")] }, signals: ["connect 15s 超时"] }))).toContain("E-02");
    expect(fired(ctx({ doctor: { ...base, checks: [chk("CHK-03", "fail")] }, signals: ["127.0.0.1:16384 offline"] }))).toContain("E-03");
    // 无对应信号时不误报
    expect(fired(ctx({ doctor: { ...base, checks: [chk("CHK-03", "fail")] } }))).not.toContain("E-02");
  });

  it("S-05 假绿灯：声称运行但设备端未实测监听", () => {
    const cards = fired(
      ctx({
        serverStatus: {
          client_version: "17.19.0",
          client_error: null,
          device_serial: "127.0.0.1:16384",
          device_server_present: false,
          device_server_version: null,
          server_running: true,
          forward_established: null,
          matrix: [],
          port: 27042,
          overall: "warn",
        },
      }),
    );
    expect(cards).toContain("S-05");
  });

  it("P-06：探针 error 态出卡；P-01 类名错误补探针级卡片", () => {
    const cards = evaluateRules(
      ctx({
        probes: [
          probe({ id: "px", status: "error", lastError: "java.lang.ClassNotFoundException: com.foo.Bar" }),
        ],
      }),
    );
    const ids = cards.map((c) => c.ruleId);
    expect(ids).toContain("P-06");
    expect(ids).toContain("P-01");
    const perProbe = cards.find((c) => c.id === "P-01:px");
    expect(perProbe).toBeTruthy();
    expect(perProbe?.cause).toContain("ClassNotFoundException");
  });

  it("O-03：active 零命中持续 15s 才出卡（未满不出）", () => {
    const since = new Map([["p1", 1_000_000 - 16_000]]);
    expect(fired(ctx({ probes: [probe({})], zeroHitSince: since }))).toContain("O-03");
    const fresh = new Map([["p1", 1_000_000 - 1_000]]);
    expect(fired(ctx({ probes: [probe({})], zeroHitSince: fresh }))).not.toContain("O-03");
    // 有命中不出卡
    const hit = new Map([["p1", 1_000_000 - 16_000]]);
    expect(fired(ctx({ probes: [probe({ hits: 3 })], zeroHitSince: hit }))).not.toContain("O-03");
  });

  it("P-05：waiting 态持续 10s 出卡", () => {
    const since = new Map([["p1", 1_000_000 - 11_000]]);
    expect(fired(ctx({ probes: [probe({ status: "waiting" })], waitingSince: since }))).toContain("P-05");
  });

  it("P-07：无 Java 层（hello.java=null）且挂了探针 → 提示 native", () => {
    const cards = fired(
      ctx({
        session: {
          phase: "running",
          evidence: [],
          device: "s",
          target: "pid:1",
          session_id: 1,
          script_id: 1,
          hello: { frida: "17.19.0", java: null },
          channel: "B",
          updated_at: "",
        },
        probes: [probe({})],
      }),
    );
    expect(cards).toContain("P-07");
  });

  it("C-07：爆破自测失败出阻断卡；C-06：引擎选 C 骨架出提示卡", () => {
    expect(fired(ctx({ bruteSelfTestFailed: true }))).toContain("C-07");
    expect(fired(ctx({ bruteEngineGenerateC: true }))).toContain("C-06");
  });

  it("D-01：回灌 push 被拒信号出卡", () => {
    expect(fired(ctx({ signals: ["adb push: Permission denied"] }))).toContain("D-01");
  });

  it("无谓词的知识库条目（如 X 组）永不触发", () => {
    expect(fired(ctx({ signals: ["ED 0C ED DA bbolt"] }))).not.toContain("X-01");
  });

  it("卡片按严重度排序：阻断在警告前", () => {
    const cards = evaluateRules(
      ctx({
        bruteSelfTestFailed: true,
        probes: [probe({ status: "waiting" })],
        waitingSince: new Map([["p1", 1_000_000 - 20_000]]),
      }),
    );
    const sev = cards.map((c) => c.severity);
    const blockIdx = sev.indexOf("block");
    const warnIdx = sev.indexOf("warn");
    expect(blockIdx).toBeGreaterThanOrEqual(0);
    expect(warnIdx).toBeGreaterThanOrEqual(0);
    expect(blockIdx).toBeLessThan(warnIdx);
  });
});
