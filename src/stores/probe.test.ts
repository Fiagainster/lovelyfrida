import { beforeEach, describe, expect, it } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { TRACE_CAP, useProbeStore } from "@/stores/probe";
import type { TraceRecord } from "@/api";

/** probe store 纯逻辑单测（A3）：trace 环形缓冲纪律（O-02） */

function rec(seq: number): TraceRecord {
  return { seq, wall: "12:00:00", run_id: "r", payload: { t: "probe_hit" } };
}

describe("probe store · trace 环形缓冲", () => {
  beforeEach(() => setActivePinia(createPinia()));

  it(`上限 ${TRACE_CAP}：新条目在前，最旧的被挤出`, () => {
    const probe = useProbeStore();
    for (let i = 0; i < TRACE_CAP + 5; i++) probe.pushTrace(rec(i));
    expect(probe.trace.length).toBe(TRACE_CAP);
    expect(probe.trace[0].seq).toBe(TRACE_CAP + 4);
    expect(probe.trace[TRACE_CAP - 1].seq).toBe(5);
  }, 30_000);

  it("clearTrace 清空视图（jsonl 全量不受影响是后端职责）", () => {
    const probe = useProbeStore();
    probe.pushTrace(rec(1));
    probe.pushTrace(rec(2));
    probe.clearTrace();
    expect(probe.trace.length).toBe(0);
  });
});
