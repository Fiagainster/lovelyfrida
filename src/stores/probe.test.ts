import { beforeEach, describe, expect, it, vi } from "vitest";
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

  it("批量 ingest：200ms flush 合并为一次数组重建，新条目在前", () => {
    vi.useFakeTimers();
    try {
      const probe = useProbeStore();
      const before = probe.trace;
      probe.ingestTrace([rec(1), rec(2), rec(3)]);
      // flush 前：视图不变（渲染节流的关键），事件先攒 pending
      expect(probe.trace).toBe(before);
      vi.advanceTimersByTime(200);
      expect(probe.trace.map((r) => r.seq)).toEqual([3, 2, 1]);
    } finally {
      vi.useRealTimers();
    }
  });

  it("批量洪峰保护：pending 达到环上限立即 flush，总量不超 TRACE_CAP", () => {
    vi.useFakeTimers();
    try {
      const probe = useProbeStore();
      const flood = Array.from({ length: TRACE_CAP + 10 }, (_, i) => rec(i));
      probe.ingestTrace(flood);
      expect(probe.trace.length).toBeLessThanOrEqual(TRACE_CAP);
      expect(probe.trace[0].seq).toBe(TRACE_CAP + 9);
    } finally {
      vi.useRealTimers();
    }
  });
});
