import { defineStore } from "pinia";
import { ref } from "vue";
import { api, type ProbeDecl, type ProbeStat, type TraceRecord } from "@/api";

/** M2：探针 / trace / 诊断（症状→处置卡片）状态 */
export const useProbeStore = defineStore("probe", () => {
  const probes = ref<ProbeStat[]>([]);
  const statsLoading = ref(false);
  const adding = ref(false);
  const rpcError = ref<string | null>(null);

  const trace = ref<TraceRecord[]>([]);
  const TRACE_CAP = 3000;

  const diagnostics = ref<{ id: string; title: string; cause: string; fix: string; rule: string }[]>([]);

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
        diagnostics.value.push({
          id: "diag-add-" + Date.now().toString(36),
          title: `探针挂载失败：${decl.clazz}.${decl.method}`,
          cause: res.error ?? "未知错误",
          fix: "先在探索器确认类名/方法名；加固壳请在 REPL 切换 classFactory.loader（P-05）",
          rule: "P-01/P-02/P-05",
        });
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
      await evaluateDiagnostics();
    } catch (e) {
      rpcError.value = String(e);
    } finally {
      statsLoading.value = false;
    }
  }

  // ---------- trace ----------
  function pushTrace(rec: TraceRecord) {
    trace.value.unshift(rec);
    if (trace.value.length > TRACE_CAP) trace.value.pop();
  }

  function clearTrace() {
    trace.value = [];
  }

  // ---------- 诊断 v1（E/S/P 子集，状态谓词触发） ----------
  let activeSince = new Map<string, number>();

  async function evaluateDiagnostics() {
    const cards: typeof diagnostics.value = [];
    const now = Date.now();
    for (const p of probes.value) {
      if (p.status === "error") {
        cards.push({
          id: "diag-err-" + p.id,
          title: `探针挂载失败：${p.clazz}.${p.method}`,
          cause: p.lastError ?? "未知错误",
          fix: "在探索器确认类名/方法名存在；内嵌类用 Outer$Inner 写法（P-01/P-02）",
          rule: "P-06 三态上报",
        });
      } else if (p.status === "active" && p.hits === 0) {
        const since = activeSince.get(p.id) ?? now;
        activeSince.set(p.id, since);
        if (now - since > 15000) {
          cards.push({
            id: "diag-zero-" + p.id,
            title: `探针零命中：${p.clazz}.${p.method}`,
            cause: "探针已挂上但从未触发——最常见原因是代码路径没走到（登录态已存在、入口手势未触发）或类被加固壳换加载器加载",
            fix: "1) 确认 App 已进入会走该方法的页面；2) 用 javaChoose 看实例是否存在；3) 在 REPL 用 javaLoaders+useLoader 切换加载器（O-03/P-05）",
            rule: "O-03 零命中",
          });
        }
      } else {
        activeSince.delete(p.id);
      }
    }
    diagnostics.value = cards;
  }

  function bindEvents(listen: typeof import("@tauri-apps/api/event").listen) {
    void listen<TraceRecord>("trace-event", (e) => {
      pushTrace(e.payload);
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
    diagnostics,
    rpc,
    addProbe,
    removeProbe,
    refreshStats,
    pushTrace,
    clearTrace,
    bindEvents,
  };
});
