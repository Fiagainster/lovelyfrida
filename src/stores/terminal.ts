import { defineStore } from "pinia";
import { ref } from "vue";
import { api, type TerminalInfo } from "@/api";

/** 终端抽屉状态（文档03：终端属于每个视图的可升降抽屉） */
export const useTerminalStore = defineStore("terminal", () => {
  const sessions = ref<TerminalInfo[]>([]);
  const activeId = ref<number | null>(null);
  const drawerOpen = ref(false);
  const lastSerial = ref("127.0.0.1:15555");

  // id → 前端 xterm 数据回调（组件挂载时注册）
  const dataHandlers = new Map<number, (b64: string) => void>();
  const closedHandlers = new Map<number, () => void>();

  function onData(id: number, cb: (b64: string) => void) {
    dataHandlers.set(id, cb);
  }
  function onClosed(id: number, cb: () => void) {
    closedHandlers.set(id, cb);
  }

  async function bindEvents(listen: typeof import("@tauri-apps/api/event").listen) {
    await listen<{ id: number; b64: string }>("terminal-out", (e) => {
      dataHandlers.get(e.payload.id)?.(e.payload.b64);
    });
    await listen<{ id: number }>("terminal-closed", (e) => {
      const id = e.payload.id;
      sessions.value = sessions.value.filter((s) => s.id !== id);
      closedHandlers.get(id)?.();
      dataHandlers.delete(id);
      closedHandlers.delete(id);
      if (activeId.value === id) {
        activeId.value = sessions.value[0]?.id ?? null;
        if (!activeId.value) drawerOpen.value = false;
      }
    });
  }

  async function create(serial: string) {
    lastSerial.value = serial;
    const info = await api.terminalCreate(serial);
    sessions.value.push(info);
    activeId.value = info.id;
    drawerOpen.value = true;
    return info;
  }

  async function close(id: number) {
    try {
      await api.terminalClose(id);
    } catch {
      /* 会话可能已随进程退出 */
    }
  }

  function toggle() {
    drawerOpen.value = !drawerOpen.value;
  }

  return {
    sessions,
    activeId,
    drawerOpen,
    lastSerial,
    onData,
    onClosed,
    bindEvents,
    create,
    close,
    toggle,
  };
});
