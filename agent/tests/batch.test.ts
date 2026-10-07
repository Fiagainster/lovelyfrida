import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

/** batch.ts 单测：双阈值 flush（256 条立即 / 100ms 定时）、aseq 序号、失败重排（批次⑪③）。 */

let sent: unknown[];

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
  // 模块级状态（buf/aseq/failStreak）必须在用例间隔离
  vi.resetModules();
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
    expect(msg.items).toEqual([{ t: "probe_hit", id: "p1", aseq: 1 }]);
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
    expect(msg.items).toEqual([{ t: "probe_hit", id: "b", aseq: 2 }]);
  });

  it("aseq 跨批单调递增（宿主据此检测丢失；重排不重盖）", async () => {
    const { emitEvent } = await importBatch();
    for (let i = 0; i < 300; i++) emitEvent({ t: "probe_hit", id: `p${i}` });
    vi.advanceTimersByTime(100);
    const first = sent[0] as { items: { aseq: number }[] };
    const second = sent[1] as { items: { aseq: number }[] };
    expect(first.items[0].aseq).toBe(1);
    expect(first.items[255].aseq).toBe(256);
    expect(second.items[0].aseq).toBe(257);
    expect(second.items[43].aseq).toBe(300);
  });

  it("flush 失败 → 重试（退避）→ 渠道恢复后整批补发且事件顺序不变", async () => {
    const { emitEvent } = await importBatch();
    emitEvent({ t: "probe_hit", id: "x1" }); // aseq 由批量层盖章
    // 先失败一次
    const failingSend = (globalThis as unknown as { send: unknown }).send;
    (globalThis as unknown as { send: unknown }).send = () => {
      throw new Error("script is destroyed");
    };
    vi.advanceTimersByTime(100); // flush → 失败 → 重排 + 安排 100ms 重试
    expect(sent).toHaveLength(0);
    // 渠道恢复
    (globalThis as unknown as { send: unknown }).send = failingSend as (p: unknown) => void;
    vi.advanceTimersByTime(100); // 重试 flush 成功
    expect(sent).toHaveLength(1);
    const msg = sent[0] as { items: { id: string; aseq: number }[] };
    expect(msg.items.map((i) => i.id)).toEqual(["x1"]);
    expect(msg.items[0].aseq).toBe(1); // 原 aseq 保留，不重盖
  });

  it("连续 3 次 flush 失败后放弃重试（渠道已死不再无限循环）", async () => {
    const { emitEvent, _pendingForTest } = await importBatch();
    emitEvent({ t: "probe_hit", id: "dead" });
    (globalThis as unknown as { send: unknown }).send = () => {
      throw new Error("script is destroyed");
    };
    vi.advanceTimersByTime(100); // 第 1 次失败 → 重试 1
    vi.advanceTimersByTime(200); // 第 2 次失败 → 重试 2
    vi.advanceTimersByTime(300); // 第 3 次失败 → 放弃（不再有定时器）
    expect(_pendingForTest()).toBe(1); // 事件仍在缓冲（进程内无处可写）
    vi.advanceTimersByTime(10000); // 不应再触发任何 flush
    expect(sent).toHaveLength(0);
  });
});
