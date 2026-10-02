<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { NButton, NModal, NTag } from "naive-ui";
import { PauseOutline, PlayOutline, TrashOutline } from "@vicons/ionicons5";
import { useProbeStore } from "@/stores/probe";
import { useSessionStore } from "@/stores/session";
import type { TraceRecord } from "@/api";

/** 时间轴视图（文档03）：探针触发流，虚拟化表格 + 参数下钻 */
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
  probe.trace.slice(0, visibleCount.value).map((r: TraceRecord) => {
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

// 渐进加载（O-02）：DOM 只渲染前 N 条，「加载更多」扩窗；全量始终在内存环 + jsonl
const visibleCount = ref(300);
const hasMore = computed(() => probe.trace.length > visibleCount.value);

function loadMore() {
  visibleCount.value += 500;
}

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

// 真暂停：事件照常写入 probe.trace 环形缓冲（不丢数据），仅冻结本视图渲染
const paused = ref(false);
const frozenRows = ref<TimelineRow[]>([]);

function togglePause() {
  if (!paused.value) frozenRows.value = liveRows.value;
  paused.value = !paused.value;
}

const rows = computed(() => (paused.value ? frozenRows.value : liveRows.value));

const detail = ref<(typeof rows.value)[number] | null>(null);

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

    <table class="plain-table timeline-table">
      <thead>
        <tr>
          <th style="width: 90px">时间</th>
          <th style="width: 80px">类型</th>
          <th>目标</th>
          <th>参数</th>
          <th style="width: 70px">线程</th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="r in rows" :key="r.seq" style="cursor: pointer" @click="detail = r">
          <td class="mono" style="font-size: 11px">{{ r.wall }}</td>
          <td>
            <span :class="['status-chip', r.kind === 'PROBE_HIT' ? 'status-chip--pass' : r.kind === 'PROBE_ERROR' ? 'status-chip--fail' : 'status-chip--running']">
              {{ r.kind }}
            </span>
          </td>
          <td class="mono" style="font-size: 12px">{{ r.target }}</td>
          <td style="font-size: 11px; color: var(--text-2); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 520px">
            {{ argsSummary(r.args) }}
          </td>
          <td class="mono" style="font-size: 11px">{{ r.thread }}</td>
        </tr>
        <tr v-if="rows.length === 0">
          <td colspan="5" class="muted" style="padding: 20px; text-align: center">
            暂无事件。挂探针并触发目标方法后，命中会实时出现在这里。
          </td>
        </tr>
      </tbody>
    </table>
    <div v-if="hasMore" style="text-align: center; padding: 10px">
      <NButton size="small" secondary @click="loadMore">
        加载更多（已显示 {{ rows.length }} / {{ probe.trace.length }} 条）
      </NButton>
    </div>

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

<style>
.timeline-table th {
  position: sticky;
  top: 0;
  background: var(--bg-panel);
  z-index: 1;
}
</style>
