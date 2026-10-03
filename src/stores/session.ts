import { defineStore } from "pinia";
import { computed, ref } from "vue";
import { api, type FridaEventPayload, type ProcEntry, type ServerStatusReport, type SessionSnapshot, type StepReport } from "@/api";

/** 会话控制台状态（M1 通道B）：frida 环境 + 进程列表 + 会话状态机 + 消息流 */
export const useSessionStore = defineStore("session", () => {
  const serverStatus = ref<ServerStatusReport | null>(null);
  const serverLoading = ref(false);
  const installSteps = ref<StepReport[] | null>(null);
  const installLoading = ref(false);

  const processes = ref<ProcEntry[]>([]);
  const procsLoading = ref(false);
  const procsFilter = ref("");

  const session = ref<SessionSnapshot | null>(null);
  const attachLoading = ref(false);
  const messages = ref<{ ts: string; kind: string; text: string }[]>([]);
  // bindEvents 重入门（dev HMR 重跑 setup 会重复注册 Tauri 监听）
  let bound = false;

  const PHASES: { key: string; label: string }[] = [
    { key: "device_ready", label: "设备就绪" },
    { key: "server_up", label: "frida-server" },
    { key: "forwarded", label: "forward" },
    { key: "attached", label: "已附加" },
    { key: "running", label: "运行中" },
  ];

  const phaseIndex = computed(() => {
    const p = session.value?.phase ?? "idle";
    const idx = PHASES.findIndex((x) => x.key === p);
    if (idx >= 0) return idx;
    if (p === "failed") return -2;
    return -1; // idle/stopped
  });

  const filteredProcesses = computed(() => {
    const q = procsFilter.value.trim().toLowerCase();
    if (!q) return processes.value;
    return processes.value.filter(
      (p) => p.name.toLowerCase().includes(q) || String(p.pid).includes(q),
    );
  });

  async function refreshServerStatus() {
    serverLoading.value = true;
    try {
      serverStatus.value = await api.fridaServerStatus();
    } finally {
      serverLoading.value = false;
    }
  }

  async function installServer() {
    installLoading.value = true;
    try {
      installSteps.value = await api.fridaServerInstall();
      await refreshServerStatus();
    } finally {
      installLoading.value = false;
    }
  }

  async function setupForward() {
    installLoading.value = true;
    try {
      const r = await api.fridaForwardSetup();
      installSteps.value = [r];
      await refreshServerStatus();
    } finally {
      installLoading.value = false;
    }
  }

  async function refreshProcesses() {
    procsLoading.value = true;
    try {
      processes.value = await api.fridaProcesses();
    } finally {
      procsLoading.value = false;
    }
  }

  async function attach(target: number | string) {
    attachLoading.value = true;
    try {
      // 案件名随附加链路落库（P2-3）：会话归入当前案件
      const { useCaseStore } = await import("@/stores/case");
      const caseStore = useCaseStore();
      session.value = await api.fridaSessionAttach(target, caseStore.apiCaseName());
      if (session.value.phase === "running") {
        void api.fridaSessionPing();
      }
    } finally {
      attachLoading.value = false;
    }
  }

  async function detachSession() {
    attachLoading.value = true;
    try {
      session.value = await api.fridaSessionDetach();
    } finally {
      attachLoading.value = false;
    }
  }

  async function refreshSession() {
    session.value = await api.fridaSessionStatus();
  }

  function pushMessage(kind: string, text: string) {
    messages.value.unshift({
      ts: new Date().toLocaleTimeString("zh-CN", { hour12: false }),
      kind,
      text,
    });
    if (messages.value.length > 100) messages.value.pop();
  }

  /** 挂事件监听；返回清理函数（App onUnmounted 调用；重入直接返回空清理，防 HMR 重复注册） */
  async function bindEvents(listen: typeof import("@tauri-apps/api/event").listen): Promise<() => void> {
    if (bound) return () => {};
    bound = true;
    const unlisten: Array<() => void> = [];
    unlisten.push(
      await listen<FridaEventPayload>("frida-event", (e) => {
        const { event, params } = e.payload;
        if (event === "message") {
          const p = params as { kind?: string; payload?: { t?: string; items?: unknown[]; [k: string]: unknown } };
          if (p.payload?.t === "pong") return; // ping 自检不刷屏
          if (p.payload?.t === "batch") {
            // agent 批量层：控制台流给摘要，明细在时间轴
            pushMessage(p.kind ?? "send", `[批量] ${p.payload.items?.length ?? 0} 条观测事件（明细见时间轴）`);
            return;
          }
          pushMessage(p.kind ?? "send", JSON.stringify(p.payload ?? p));
        } else if (event === "detached") {
          const p = params as { reason?: string };
          pushMessage("detached", p.reason ?? "detached");
          void refreshSession();
        }
        // S 组谓词依赖会话证据/消息流（detached、端口切换等信号）
        void import("@/stores/diagnostics").then(({ useDiagStore }) => useDiagStore().scheduleReevaluate());
      }),
    );
    unlisten.push(
      await listen<SessionSnapshot>("session-state", (e) => {
        session.value = e.payload;
        void import("@/stores/diagnostics").then(({ useDiagStore }) => useDiagStore().scheduleReevaluate());
      }),
    );
    return () => {
      bound = false;
      while (unlisten.length) unlisten.pop()?.();
    };
  }

  return {
    serverStatus,
    serverLoading,
    installSteps,
    installLoading,
    processes,
    procsLoading,
    procsFilter,
    filteredProcesses,
    session,
    attachLoading,
    messages,
    PHASES,
    phaseIndex,
    refreshServerStatus,
    installServer,
    setupForward,
    refreshProcesses,
    attach,
    detachSession,
    refreshSession,
    pushMessage,
    bindEvents,
  };
});
