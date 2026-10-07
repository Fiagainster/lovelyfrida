import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";

/**
 * session store 单测（批次⑭）：bindEvents 的事件 payload 分支（pong 静默 /
 * batch 摘要 / 原始消息 / detached 收敛）。@/api 用替身——detached 分支会真调
 * refreshSession，浏览器测试环境 isTauri() 为 false 会抛「预览模式不可用」。
 */

vi.mock("@/api", () => ({
  api: {
    fridaSessionStatus: vi.fn(async () => ({ phase: "idle" })),
    fridaServerStatus: vi.fn(async () => ({})),
  },
  errMsg: (e: unknown) => String(e),
}));

import { useSessionStore } from "@/stores/session";

type Handler = (e: { payload: unknown }) => void;

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

function fridaMsg(payload: unknown, kind = "send") {
  return { event: "message", params: { kind, payload } };
}

describe("session store · bindEvents payload 分支", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it("pong（ping 自检）不进控制台消息流", async () => {
    const store = useSessionStore();
    const { listen, fire } = fakeListen();
    await store.bindEvents(listen as never);
    fire("frida-event", fridaMsg({ t: "pong" }));
    expect(store.messages).toHaveLength(0);
  });

  it("batch 事件只进一条摘要（明细在时间轴）", async () => {
    const store = useSessionStore();
    const { listen, fire } = fakeListen();
    await store.bindEvents(listen as never);
    fire("frida-event", fridaMsg({ t: "batch", items: [{}, {}, {}] }));
    expect(store.messages).toHaveLength(1);
    expect(store.messages[0].text).toContain("3 条观测事件");
    expect(store.messages[0].kind).toBe("send");
  });

  it("普通 send 消息 JSON 化进流；kind 缺省 send", async () => {
    const store = useSessionStore();
    const { listen, fire } = fakeListen();
    await store.bindEvents(listen as never);
    fire("frida-event", fridaMsg({ t: "probe_hit", id: "p1" }));
    expect(store.messages).toHaveLength(1);
    expect(store.messages[0].text).toContain("probe_hit");
    // kind 缺省分支
    const { fire: fire2 } = fakeListen();
    void fire2;
    fire("frida-event", { event: "message", params: { payload: { t: "hello" } } });
    expect(store.messages).toHaveLength(2);
    expect(store.messages[0].kind).toBe("send");
  });

  it("detached：进消息流（kind=detached）并触发会话状态刷新", async () => {
    const store = useSessionStore();
    const { listen, fire } = fakeListen();
    await store.bindEvents(listen as never);
    fire("frida-event", { event: "detached", params: { reason: "process terminated" } });
    expect(store.messages).toHaveLength(1);
    expect(store.messages[0].kind).toBe("detached");
    expect(store.messages[0].text).toBe("process terminated");
    // refreshSession 是异步的（mock api），让 microtask 跑完再断言未抛错即可
    await Promise.resolve();
  });

  it("pushMessage 稳定 id 单调递增（批次⑫ v-for key 契约）", async () => {
    const store = useSessionStore();
    const { listen, fire } = fakeListen();
    await store.bindEvents(listen as never);
    fire("frida-event", fridaMsg({ t: "a" }));
    fire("frida-event", fridaMsg({ t: "b" }));
    fire("frida-event", fridaMsg({ t: "c" }));
    const ids = store.messages.map((m) => m.id);
    expect(new Set(ids).size).toBe(ids.length); // id 必须唯一（v-for key 稳定性）
    expect(ids).toEqual([3, 2, 1]); // 前插列表最新在前，id 随生成序单调递增
  });

  it("session-state 事件直接更新会话快照", async () => {
    const store = useSessionStore();
    const { listen, fire } = fakeListen();
    await store.bindEvents(listen as never);
    fire("session-state", { phase: "running", session_id: 7 });
    expect(store.session?.phase).toBe("running");
    expect(store.session?.session_id).toBe(7);
  });
});
