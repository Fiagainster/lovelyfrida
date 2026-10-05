import { defineStore } from "pinia";
import { computed, ref, shallowRef } from "vue";
import { api, type ProbeDecl, type ProbeStat, type TraceRecord } from "@/api";
import { useDiagStore } from "@/stores/diagnostics";
import { useCaseStore } from "@/stores/case";
import { parseSymbols, type StaticSymbol } from "@/utils/symbols";

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

  // bindEvents 重入门（dev HMR 重跑 setup 会重复注册 Tauri 监听）
  let bound = false;

  // ---------- 静态符号候选（D6/B3 落地半边） ----------
  // jadx 等静态工具导出的目标方法清单 → 探针候选（一键预填）。按案件分桶 localStorage 持久化。
  const SYMBOLS_KEY = "lovelyfrida.staticSymbols";
  const symbolsByCase = ref<Record<string, StaticSymbol[]>>(loadSymbols());

  function loadSymbols(): Record<string, StaticSymbol[]> {
    try {
      const raw = localStorage.getItem(SYMBOLS_KEY);
      return raw ? (JSON.parse(raw) as Record<string, StaticSymbol[]>) : {};
    } catch {
      return {};
    }
  }

  /** 当前案件的静态符号候选（案件名来自 case 单一真源） */
  const staticSymbols = computed<StaticSymbol[]>(
    () => symbolsByCase.value[useCaseStore().apiCaseName()] ?? [],
  );

  function persistSymbols() {
    localStorage.setItem(SYMBOLS_KEY, JSON.stringify(symbolsByCase.value));
  }

  /** 导入静态符号文本（点号行 / JSON 数组），返回新增条数（与已有去重） */
  function importSymbols(text: string): number {
    const parsed = parseSymbols(text);
    if (!parsed.length) return 0;
    const k = useCaseStore().apiCaseName();
    const existing = symbolsByCase.value[k] ?? [];
    const seen = new Set(existing.map((s) => `${s.clazz}#${s.method}`));
    const merged = [...existing, ...parsed.filter((s) => !seen.has(`${s.clazz}#${s.method}`))];
    symbolsByCase.value = { ...symbolsByCase.value, [k]: merged };
    persistSymbols();
    return merged.length - existing.length;
  }

  function removeSymbol(index: number) {
    const k = useCaseStore().apiCaseName();
    const list = [...(symbolsByCase.value[k] ?? [])];
    list.splice(index, 1);
    symbolsByCase.value = { ...symbolsByCase.value, [k]: list };
    persistSymbols();
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

  // 高频路径（trace-event）走 200ms 批量 flush：pushTrace 的 O(cap) 数组重建
  // 在千级事件/s 下本身就是渲染瓶颈。落盘不受影响（后端逐条全量 jsonl）。
  const pendingTrace: TraceRecord[] = [];
  let traceFlushTimer: ReturnType<typeof setTimeout> | null = null;

  function flushTrace() {
    if (traceFlushTimer !== null) {
      clearTimeout(traceFlushTimer);
      traceFlushTimer = null;
    }
    if (!pendingTrace.length) return;
    const batch = pendingTrace.splice(0, pendingTrace.length);
    batch.reverse(); // 到达序 → 新在前
    trace.value = [...batch, ...trace.value].slice(0, TRACE_CAP);
    diag().scheduleReevaluate();
  }

  function ingestTrace(recs: TraceRecord[]) {
    for (const r of recs) pendingTrace.push(r);
    if (pendingTrace.length >= TRACE_CAP) {
      flushTrace(); // 洪峰保护：pending 不超过环本身
      return;
    }
    if (traceFlushTimer === null) {
      traceFlushTimer = setTimeout(flushTrace, 200);
    }
  }

  function clearTrace() {
    pendingTrace.length = 0;
    trace.value = [];
  }

  /** 挂事件监听；返回清理函数（App onUnmounted 调用；重入直接返回空清理，防 HMR 重复注册） */
  async function bindEvents(listen: typeof import("@tauri-apps/api/event").listen): Promise<() => void> {
    if (bound) return () => {};
    bound = true;
    const unlisten: Array<() => void> = [];
    unlisten.push(
      await listen<TraceRecord>("trace-event", (e) => {
        const p = e.payload as Partial<TraceRecord> & { t?: string; records?: TraceRecord[] };
        // agent 批量层：后端单次 emit 携带 {t:"trace_batch", records:[TraceRecord...]}
        if (p?.t === "trace_batch" && Array.isArray(p.records)) {
          ingestTrace(p.records);
          return;
        }
        ingestTrace([p as TraceRecord]);
        // 高频事件走节流重算（P/O 组谓词依赖探针状态与 trace 变化）——flush 时统一触发
      }),
    );
    // dumpDex 落盘事件（此前后端有发前端无收）：作为一条 trace 进入时间轴
    unlisten.push(
      await listen<{ path: string; size: number; base: string }>("dex-dumped", (e) => {
        pushTrace({
          seq: Date.now(),
          wall: new Date().toLocaleTimeString("zh-CN", { hour12: false }),
          run_id: "dex-dump",
          payload: { t: "dex_dumped", ...e.payload },
        });
      }),
    );
    return () => {
      bound = false;
      while (unlisten.length) unlisten.pop()?.();
    };
  }

  return {
    probes,
    statsLoading,
    adding,
    rpcError,
    trace,
    staticSymbols,
    rpc,
    addProbe,
    removeProbe,
    refreshStats,
    importSymbols,
    removeSymbol,
    pushTrace,
    ingestTrace,
    flushTrace,
    clearTrace,
    bindEvents,
  };
});
