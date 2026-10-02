import { defineStore } from "pinia";
import { ref, shallowRef } from "vue";
import { api, type ProbeDecl, type ProbeStat, type TraceRecord } from "@/api";
import { useDiagStore } from "@/stores/diagnostics";

/** trace 内存环上限（O-02；全量落 cases/traces/*.jsonl，库内只存索引） */
export const TRACE_CAP = 3000;

/** M2：探针 / trace / 诊断（症状→处置卡片）状态 */
export const useProbeStore = defineStore("probe", () => {
  const probes = ref<ProbeStat[]>([]);
  const statsLoading = ref(false);
  const adding = ref(false);
  const rpcError = ref<string | null>(null);

  // shallowRef：事件高频时深响应代理的逐元素 trap 是 O-02 卡顿根源——
  // 改为整体重建数组（O(n) 指针拷贝 + 单次触发），消费端 computed 正常响应
  const trace = shallowRef<TraceRecord[]>([]);

  // 诊断卡片改由规则引擎驱动（P2-1）：stores/diagnostics.ts + diagnostics/rules.ts
  function diag() {
    return useDiagStore();
  }

  // ---------- RPC ----------
  async function rpc<T>(f: string, args: unknown[]): Promise<T> {
    rpcError.value = null;
    try {
      return (await api.fridaRpc(f, args)) as T;
    } catch (e) {
      rpcError.value = String(e);
      throw e;
    }
  }

  async function addProbe(decl: Omit<ProbeDecl, "id">): Promise<string> {
    adding.value = true;
    try {
      const id = "p-" + Date.now().toString(36);
      const r = await rpc<{ results: { id: string; status: string; error: string | null }[] }>(
        "addProbes",
        [{ probes: [{ ...decl, id }] }],
      );
      const res = r.results[0];
      if (res.status === "error") {
        diag().scheduleReevaluate();
        throw new Error(res.error ?? "挂载失败");
      }
      await refreshStats();
      return res.status === "waiting" ? "waiting" : "active";
    } finally {
      adding.value = false;
    }
  }

  async function removeProbe(id: string) {
    await rpc("removeProbes", [{ ids: [id] }]);
    await refreshStats();
  }

  async function refreshStats() {
    statsLoading.value = true;
    try {
      probes.value = await rpc<ProbeStat[]>("probeStats", []);
      diag().scheduleReevaluate();
    } catch (e) {
      rpcError.value = String(e);
    } finally {
      statsLoading.value = false;
    }
  }

  // ---------- trace ----------
  function pushTrace(rec: TraceRecord) {
    trace.value = [rec, ...trace.value.slice(0, TRACE_CAP - 1)];
  }

  function clearTrace() {
    trace.value = [];
  }

  function bindEvents(listen: typeof import("@tauri-apps/api/event").listen) {
    void listen<TraceRecord>("trace-event", (e) => {
      pushTrace(e.payload);
      // 高频事件走节流重算（P/O 组谓词依赖探针状态与 trace 变化）
      diag().scheduleReevaluate();
    });
    // dumpDex 落盘事件（此前后端有发前端无收）：作为一条 trace 进入时间轴
    void listen<{ path: string; size: number; base: string }>("dex-dumped", (e) => {
      pushTrace({
        seq: Date.now(),
        wall: new Date().toLocaleTimeString("zh-CN", { hour12: false }),
        run_id: "dex-dump",
        payload: { t: "dex_dumped", ...e.payload },
      });
    });
  }

  return {
    probes,
    statsLoading,
    adding,
    rpcError,
    trace,
    rpc,
    addProbe,
    removeProbe,
    refreshStats,
    pushTrace,
    clearTrace,
    bindEvents,
  };
});
