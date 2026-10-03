<script setup lang="ts">
import { computed, h, onMounted, ref } from "vue";
import { NButton, NDataTable, NModal, NTag, type DataTableColumns } from "naive-ui";
import { PauseOutline, PlayOutline, TrashOutline } from "@vicons/ionicons5";
import { useProbeStore } from "@/stores/probe";
import { useSessionStore } from "@/stores/session";
import type { TraceRecord } from "@/api";

/** 时间轴视图（文档03）：探针触发流，NDataTable 虚拟滚动（O-02：DOM 只渲染视口行）+ 参数下钻 */
const probe = useProbeStore();
const session = useSessionStore();

interface TimelineRow {
  seq: number;
  wall: string;
  kind: string;
  target: string;
  args: { k: string; v: string }[];
  ret: { k: string; v: string } | null;
  thread: number;
  raw: unknown;
}

const liveRows = computed<TimelineRow[]>(() =>
  probe.trace.map((r: TraceRecord) => {
    const p = r.payload;
    const args = (p.args as { k: string; v: string }[] | undefined) ?? [];
    return {
      seq: r.seq,
      wall: r.wall,
      kind: String(p.t ?? "").toUpperCase(),
      target: `${String(p.clazz ?? "")}.${String(p.method ?? "")}`,
      args,
      ret: (p.ret as { k: string; v: string } | undefined) ?? null,
      thread: Number(p.thread ?? 0),
      raw: p,
    };
  }),
);

// 真暂停：事件照常写入 probe.trace 环形缓冲（不丢数据），仅冻结本视图渲染
const paused = ref(false);
const frozenRows = ref<TimelineRow[]>([]);

function togglePause() {
  if (!paused.value) frozenRows.value = liveRows.value;
  paused.value = !paused.value;
}

const rows = computed(() => (paused.value ? frozenRows.value : liveRows.value));

function kindChipClass(kind: string): string {
  if (kind === "PROBE_HIT") return "status-chip--pass";
  if (kind === "PROBE_ERROR" || kind === "TRACE_GAP") return "status-chip--fail";
  return "status-chip--running";
}

// 虚拟滚动列（O-02 承诺兑现：此前是渐进窗口 + 普通 table，千级事件 diff 全量 vnode）
const columns: DataTableColumns<TimelineRow> = [
  {
    title: "时间",
    key: "wall",
    width: 90,
    render: (r) => h("span", { class: "mono", style: "font-size: 11px" }, r.wall),
  },
  {
    title: "类型",
    key: "kind",
    width: 110,
    render: (r) => h("span", { class: ["status-chip", kindChipClass(r.kind)] }, r.kind),
  },
  {
    title: "目标",
    key: "target",
    width: 320,
    ellipsis: { tooltip: true },
    render: (r) => h("span", { class: "mono", style: "font-size: 12px" }, r.target),
  },
  {
    title: "参数",
    key: "args",
    ellipsis: { tooltip: true },
    render: (r) => h("span", { style: "font-size: 11px; color: var(--text-2)" }, argsSummary(r.args)),
  },
  {
    title: "线程",
    key: "thread",
    width: 70,
    render: (r) => h("span", { class: "mono", style: "font-size: 11px" }, String(r.thread)),
  },
];

function download(name: string, content: string, mime: string) {
  const blob = new Blob([content], { type: mime });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = name;
  a.click();
  URL.revokeObjectURL(url);
}

function tsStamp(): string {
  const d = new Date();
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}${p(d.getMonth() + 1)}${p(d.getDate())}-${p(d.getHours())}${p(d.getMinutes())}${p(d.getSeconds())}`;
}

function csvCell(v: string): string {
  return `"${v.replace(/"/g, '""')}"`;
}

function exportCsv() {
  const head = "seq,wall,kind,target,thread,args,ret";
  const lines = probe.trace.map((r: TraceRecord) => {
    const p = r.payload;
    const args = (p.args as { k: string; v: string }[] | undefined) ?? [];
    const ret = (p.ret as { k: string; v: string } | undefined) ?? null;
    return [
      String(r.seq),
      r.wall,
      String(p.t ?? ""),
      `${String(p.clazz ?? "")}.${String(p.method ?? "")}`,
      String(p.thread ?? ""),
      args.map((a) => `${a.k}:${a.v}`).join(" | "),
      ret ? `${ret.k}:${ret.v}` : "",
    ]
      .map(csvCell)
      .join(",");
  });
  download(`timeline-${tsStamp()}.csv`, "\uFEFF" + [head, ...lines].join("\n"), "text/csv;charset=utf-8");
}

function exportJson() {
  download(`timeline-${tsStamp()}.json`, JSON.stringify(probe.trace, null, 2), "application/json");
}

const detail = ref<TimelineRow | null>(null);

function argsSummary(args: { k: string; v: string }[]): string {
  return args.map((a) => `${a.k}:${a.v}`).join(" ｜ ").slice(0, 180);
}

onMounted(() => {
  // 时间轴打开时拉一次探针状态（联动零命中诊断）
  if (session.session?.phase === "running") void probe.refreshStats();
});
</script>

<template>
  <div class="workspace__view-inner" style="max-width: none">
    <div class="view-head">
      <div class="view-head__title">
        <div class="view-head__icon" style="background: var(--accent-soft)">⏱</div>
        <div>
          <h2>时间轴</h2>
          <div class="view-head__sub">
            探针触发流 · {{ probe.trace.length }} 条（内存保留 3000，全量落
            cases/traces/*.jsonl）· 点行下钻
            <span v-if="paused" style="color: var(--warn, #e3b341)">· 已暂停（后台仍在记录，点「继续」恢复实时）</span>
          </div>
        </div>
      </div>
      <div class="view-head__actions">
        <NButton size="small" secondary @click="togglePause">
          <template #icon><component :is="paused ? PlayOutline : PauseOutline" /></template>
          {{ paused ? "继续" : "暂停" }}
        </NButton>
        <NButton size="small" quaternary :disabled="probe.trace.length === 0" @click="exportCsv">导出 CSV</NButton>
        <NButton size="small" quaternary :disabled="probe.trace.length === 0" @click="exportJson">导出 JSON</NButton>
        <NButton size="small" quaternary type="warning" @click="probe.clearTrace()">
          <template #icon><TrashOutline /></template>
          清空视图
        </NButton>
      </div>
    </div>

    <!-- 虚拟滚动：3000 条全量进 data，DOM 只渲染视口行（不再需要「加载更多」窗口） -->
    <NDataTable
      size="small"
      :columns="columns"
      :data="rows"
      :row-key="(r: TimelineRow) => r.seq"
      :row-props="(r: TimelineRow) => ({ style: 'cursor: pointer', onClick: () => (detail = r) })"
      :max-height="'calc(100vh - 300px)'"
      virtual-scroll
    >
      <template #empty>
        <span class="muted" style="padding: 20px">暂无事件。挂探针并触发目标方法后，命中会实时出现在这里。</span>
      </template>
    </NDataTable>

    <!-- 下钻 modal -->
    <NModal
      :show="detail !== null"
      preset="card"
      :title="detail ? `${detail.target} · #${detail.seq}` : ''"
      style="width: 720px"
      :bordered="false"
      size="small"
      @update:show="detail = null"
    >
      <template v-if="detail">
        <table class="plain-table">
          <tbody>
            <tr>
              <td style="width: 110px; color: var(--text-3)">时间 / 线程</td>
              <td class="mono">{{ detail.wall }} · T{{ detail.thread }}</td>
            </tr>
            <tr v-for="(a, i) in detail.args" :key="i">
              <td style="color: var(--text-3)">参数 {{ i }}</td>
              <td>
                <span :class="['status-chip', a.k === 'bytes' ? 'status-chip--running' : 'status-chip--pending']">{{ a.k }}</span>
                <code class="mono" style="margin-left: 8px; word-break: break-all">{{ a.v }}</code>
              </td>
            </tr>
            <tr v-if="detail.ret">
              <td style="color: var(--text-3)">返回值</td>
              <td>
                <span class="status-chip status-chip--pass">{{ detail.ret.k }}</span>
                <code class="mono" style="margin-left: 8px; word-break: break-all">{{ detail.ret.v }}</code>
              </td>
            </tr>
          </tbody>
        </table>
        <div style="margin-top: 10px">
          <NTag size="small" :bordered="false">原始 payload</NTag>
          <pre class="check-item__evidence" style="margin-top: 6px">{{ JSON.stringify(detail.raw, null, 2) }}</pre>
        </div>
      </template>
    </NModal>
  </div>
</template>
