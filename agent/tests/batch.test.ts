import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

/** batch.ts 单测：双阈值 flush（256 条立即 / 100ms 定时）与消息形态。 */

let sent: unknown[] = [];

beforeEach(() => {
  sent = [];
  (globalThis as unknown as { send: unknown }).send = (p: unknown) => {
    sent.push(p);
  };
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

async function importBatch() {
  const mod = await import("../src/batch");
  return mod;
}

describe("agent batch 层", () => {
  it("单条事件不立即 send：等 100ms 定时器合并为 {t:'batch', items}", async () => {
    const { emitEvent } = await importBatch();
    emitEvent({ t: "probe_hit", id: "p1" });
    expect(sent).toHaveLength(0);
    vi.advanceTimersByTime(100);
    expect(sent).toHaveLength(1);
    const msg = sent[0] as { t: string; items: unknown[] };
    expect(msg.t).toBe("batch");
    expect(msg.items).toEqual([{ t: "probe_hit", id: "p1" }]);
  });

  it("条数阈值 256：达到即立即 flush（不等到定时器），洪峰下批量大小钉死", async () => {
    const { emitEvent } = await importBatch();
    for (let i = 0; i < 256; i++) emitEvent({ t: "probe_hit", id: `p${i}` });
    expect(sent).toHaveLength(1);
    const msg = sent[0] as { t: string; items: unknown[] };
    expect(msg.items).toHaveLength(256);
    // 定时器此刻不应再触发第二次 flush
    vi.advanceTimersByTime(200);
    expect(sent).toHaveLength(1);
  });

  it("256 条阈值之上继续积累 → 下一批正好再 256 条（批量恒定）", async () => {
    const { emitEvent } = await importBatch();
    for (let i = 0; i < 300; i++) emitEvent({ t: "probe_hit", id: `p${i}` });
    expect(sent).toHaveLength(1);
    vi.advanceTimersByTime(100);
    expect(sent).toHaveLength(2);
    const second = sent[1] as { t: string; items: unknown[] };
    expect(second.items).toHaveLength(44); // 300 - 256
  });

  it("flush 后计时器复位：不重复发送空批", async () => {
    const { emitEvent } = await importBatch();
    emitEvent({ t: "probe_hit", id: "a" });
    vi.advanceTimersByTime(100);
    expect(sent).toHaveLength(1);
    vi.advanceTimersByTime(500);
    expect(sent).toHaveLength(1); // 无新事件不再发送
    emitEvent({ t: "probe_hit", id: "b" });
    vi.advanceTimersByTime(100);
    expect(sent).toHaveLength(2);
    const msg = sent[1] as { t: string; items: unknown[] };
    expect(msg.items).toEqual([{ t: "probe_hit", id: "b" }]);
  });
});
