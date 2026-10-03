import { defineStore } from "pinia";
import { ref } from "vue";
import { api, type TerminalInfo } from "@/api";

/** 终端抽屉状态（文档03：终端属于每个视图的可升降抽屉） */
export const useTerminalStore = defineStore("terminal", () => {
  const sessions = ref<TerminalInfo[]>([]);
  const activeId = ref<number | null>(null);
  const drawerOpen = ref(false);
  const lastSerial = ref("127.0.0.1:16384");
  // bindEvents 重入门（dev HMR 重跑 setup 会重复注册 Tauri 监听）
  let bound = false;

  // id → 前端 xterm 数据回调（组件挂载时注册）
  const dataHandlers = new Map<number, (b64: string) => void>();
  const closedHandlers = new Map<number, () => void>();

  function onData(id: number, cb: (b64: string) => void) {
    dataHandlers.set(id, cb);
  }
  function onClosed(id: number, cb: () => void) {
    closedHandlers.set(id, cb);
  }

  // 输出合流（O-02）：PTY 按 8KB 块逐条 emit，cat 大文件/logcat 时每块一次
  // xterm.write；这里按 id 攒 16ms 内到达的块合并为一次 write（低于一帧，无感延迟）
  const pendingOut = new Map<number, string[]>();
  const outTimers = new Map<number, ReturnType<typeof setTimeout>>();

  function flushOut(id: number) {
    const t = outTimers.get(id);
    if (t !== undefined) {
      clearTimeout(t);
      outTimers.delete(id);
    }
    const list = pendingOut.get(id);
    pendingOut.delete(id);
    if (list?.length) dataHandlers.get(id)?.(list.join(""));
  }

  function pushOut(id: number, b64: string) {
    let list = pendingOut.get(id);
    if (!list) {
      list = [];
      pendingOut.set(id, list);
    }
    list.push(b64);
    if (list.length >= 32) {
      flushOut(id); // 洪峰：攒够立即合并
      return;
    }
    if (!outTimers.has(id)) {
      outTimers.set(id, setTimeout(() => flushOut(id), 16));
    }
  }

  async function bindEvents(listen: typeof import("@tauri-apps/api/event").listen): Promise<() => void> {
    if (bound) return () => {};
    bound = true;
    const unlisten: Array<() => void> = [];
    unlisten.push(
      await listen<{ id: number; b64: string }>("terminal-out", (e) => {
        if (!dataHandlers.has(e.payload.id)) return; // 无人订阅时不必攒缓冲
        pushOut(e.payload.id, e.payload.b64);
      }),
    );
    unlisten.push(
      await listen<{ id: number }>("terminal-closed", (e) => {
        const id = e.payload.id;
        flushOut(id); // 先把攒着的尾部输出写完再清理
        sessions.value = sessions.value.filter((s) => s.id !== id);
        closedHandlers.get(id)?.();
        dataHandlers.delete(id);
        closedHandlers.delete(id);
        pendingOut.delete(id);
        if (activeId.value === id) {
          activeId.value = sessions.value[0]?.id ?? null;
          if (!activeId.value) drawerOpen.value = false;
        }
      }),
    );
    return () => {
      bound = false;
      while (unlisten.length) unlisten.pop()?.();
    };
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
