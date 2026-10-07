import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { useTerminalStore } from "@/stores/terminal";

/** terminal store 单测（批次⑭）：16ms 输出合流（O-02）+ terminal-closed 收尾语义。 */

type Handler = (e: { payload: unknown }) => void;

/** Tauri listen 的替身：捕获 handler，测试里手动 fire */
function fakeListen() {
  const handlers = new Map<string, Handler[]>();
  const listen = vi.fn(async (event: string, cb: Handler) => {
    handlers.set(event, [...(handlers.get(event) ?? []), cb]);
    return () => {
      handlers.set(event, (handlers.get(event) ?? []).filter((h) => h !== cb));
    };
  });
  const fire = (event: string, payload: unknown) => {
    for (const h of handlers.get(event) ?? []) h({ payload });
  };
  return { listen, fire };
}

describe("terminal store · 输出合流", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("16ms 内到达的多块合并为一次 write（低于一帧无感延迟）", async () => {
    const store = useTerminalStore();
    const { listen, fire } = fakeListen();
    await store.bindEvents(listen as never);
    const writes: string[] = [];
    store.onData(1, (b64) => writes.push(b64));
    fire("terminal-out", { id: 1, b64: "a" });
    fire("terminal-out", { id: 1, b64: "b" });
    fire("terminal-out", { id: 1, b64: "c" });
    expect(writes).toEqual([]); // 未到 16ms 不写
    vi.advanceTimersByTime(16);
    expect(writes).toEqual(["abc"]); // 一次合并写
  });

  it("洪峰攒满 32 块立即合并（不等定时器）", async () => {
    const store = useTerminalStore();
    const { listen, fire } = fakeListen();
    await store.bindEvents(listen as never);
    const writes: string[] = [];
    store.onData(1, (b64) => writes.push(b64));
    for (let i = 0; i < 32; i++) fire("terminal-out", { id: 1, b64: String(i) });
    expect(writes).toHaveLength(1);
    expect(writes[0]).toBe(Array.from({ length: 32 }, (_, i) => String(i)).join(""));
  });

  it("terminal-closed：尾部输出先写完，会话/activeId/抽屉状态收敛", async () => {
    const store = useTerminalStore();
    const { listen, fire } = fakeListen();
    await store.bindEvents(listen as never);
    // 两个会话，#2 是活动会话
    store.sessions.push(
      { id: 1, serial: "s1", title: "t1", created_at: "" } as never,
      { id: 2, serial: "s2", title: "t2", created_at: "" } as never,
    );
    store.activeId = 2;
    store.drawerOpen = true;
    const writes: string[] = [];
    store.onData(2, (b64) => writes.push(b64));
    const onClosed = vi.fn();
    store.onClosed(2, onClosed);
    fire("terminal-out", { id: 2, b64: "tail" });
    fire("terminal-closed", { id: 2 });
    expect(writes).toEqual(["tail"]); // 关闭时攒着的尾部输出必须先写完
    expect(onClosed).toHaveBeenCalledTimes(1);
    expect(store.sessions.map((s) => s.id)).toEqual([1]);
    expect(store.activeId).toBe(1); // 活动会话回退到剩余第一个
    expect(store.drawerOpen).toBe(true);
    // 再关最后一个：activeId 清空 + 抽屉收起
    fire("terminal-closed", { id: 1 });
    expect(store.activeId).toBeNull();
    expect(store.drawerOpen).toBe(false);
  });

  it("无人订阅的输出不攒缓冲（fire 不炸即可）", async () => {
    const store = useTerminalStore();
    const { listen, fire } = fakeListen();
    await store.bindEvents(listen as never);
    expect(() => fire("terminal-out", { id: 99, b64: "x" })).not.toThrow();
  });
});
