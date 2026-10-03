<script setup lang="ts">
import { onMounted, ref } from "vue";
import { NButton, NTabPane, NTabs, NTag, useMessage } from "naive-ui";
import { RefreshOutline } from "@vicons/ionicons5";
import {
  api,
  type HistoryBruteJob,
  type HistoryExperiment,
  type HistoryRun,
  type HistorySession,
} from "@/api";

/**
 * B2 落库数据面（文档10）：P2-3 写进 cases.db 的会话/trace run/实验/爆破作业，
 * 在这里交还给 UI——归档节点的「历史档案」。只读查询 + 刷新。
 */
const message = useMessage();
const tab = ref("sessions");
const loading = ref(false);

const sessions = ref<HistorySession[]>([]);
const runs = ref<HistoryRun[]>([]);
const experiments = ref<HistoryExperiment[]>([]);
const bruteJobs = ref<HistoryBruteJob[]>([]);

async function refresh() {
  loading.value = true;
  try {
    const [s, r, e, b] = await Promise.all([
      api.historySessions(30),
      api.historyRuns(30),
      api.historyExperiments(30),
      api.historyBruteJobs(30),
    ]);
    sessions.value = s;
    runs.value = r;
    experiments.value = e;
    bruteJobs.value = b;
  } catch (err) {
    message.error(String(err));
  } finally {
    loading.value = false;
  }
}

function fmtTs(iso: string): string {
  // RFC3339 → 本地可读（截秒）
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString("zh-CN", { hour12: false });
}

onMounted(refresh);
</script>

<template>
  <div class="card info-card">
    <h3>
      历史档案（cases.db 数据面）
      <NTag size="small" :bordered="false">{{ sessions.length + runs.length + experiments.length + bruteJobs.length }} 条</NTag>
      <NButton size="tiny" quaternary :loading="loading" style="margin-left: 8px" @click="refresh">
        <template #icon><RefreshOutline /></template>
        刷新
      </NButton>
    </h3>
    <NTabs v-model:value="tab" type="segment" size="small">
      <NTabPane name="sessions" tab="会话">
        <table class="plain-table" style="margin-top: 8px">
          <thead>
            <tr><th style="width: 60px">#</th><th>案件</th><th>设备</th><th>目标</th><th style="width: 60px">通道</th><th style="width: 80px">状态</th><th>开始 / 结束</th></tr>
          </thead>
          <tbody>
            <tr v-for="s in sessions" :key="s.id">
              <td class="mono">{{ s.id }}</td>
              <td style="font-size: 12px">{{ s.case_name }}</td>
              <td class="mono" style="font-size: 11px">{{ s.device_serial ?? "—" }}</td>
              <td class="mono" style="font-size: 11px">{{ s.target || "—" }}</td>
              <td><NTag size="tiny" :bordered="false" :type="s.channel === 'C' ? 'warning' : 'info'">{{ s.channel }}</NTag></td>
              <td><span :class="['status-chip', s.state === 'running' ? 'status-chip--pass' : s.state === 'stopped' ? 'status-chip--pending' : 'status-chip--warn']">{{ s.state }}</span></td>
              <td style="font-size: 11px">{{ fmtTs(s.started_at) }}<span v-if="s.ended_at"> → {{ fmtTs(s.ended_at) }}</span></td>
            </tr>
            <tr v-if="sessions.length === 0"><td colspan="7" class="muted" style="padding: 12px; text-align: center">暂无会话记录（附加成功后自动入库）</td></tr>
          </tbody>
        </table>
      </NTabPane>

      <NTabPane name="runs" tab="Trace Run">
        <table class="plain-table" style="margin-top: 8px">
          <thead>
            <tr><th style="width: 60px">#</th><th style="width: 70px">会话</th><th style="width: 80px">状态</th><th>trace 文件</th><th style="width: 90px">行数</th><th>开始 / 结束</th></tr>
          </thead>
          <tbody>
            <tr v-for="r in runs" :key="r.id">
              <td class="mono">{{ r.id }}</td>
              <td class="mono">session#{{ r.session_id }}</td>
              <td><span :class="['status-chip', r.status === 'done' ? 'status-chip--pass' : 'status-chip--running']">{{ r.status }}</span></td>
              <td class="mono" style="font-size: 11px; max-width: 260px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap">{{ r.trace_path }}</td>
              <td class="mono">{{ r.line_count }}</td>
              <td style="font-size: 11px">{{ fmtTs(r.started_at) }}<span v-if="r.ended_at"> → {{ fmtTs(r.ended_at) }}</span></td>
            </tr>
            <tr v-if="runs.length === 0"><td colspan="6" class="muted" style="padding: 12px; text-align: center">暂无 trace run（附加成功开启时间轴后自动入库）</td></tr>
          </tbody>
        </table>
      </NTabPane>

      <NTabPane name="experiments" tab="实验">
        <table class="plain-table" style="margin-top: 8px">
          <thead>
            <tr><th style="width: 60px">#</th><th>案件</th><th>标题</th><th style="width: 80px">状态</th><th style="width: 90px">报告</th><th>时间</th></tr>
          </thead>
          <tbody>
            <tr v-for="e in experiments" :key="e.id">
              <td class="mono">{{ e.id }}</td>
              <td style="font-size: 12px">{{ e.case_name }}</td>
              <td style="font-size: 12px">{{ e.title }}</td>
              <td><NTag size="tiny" :bordered="false">{{ e.status }}</NTag></td>
              <td class="mono" style="font-size: 11px">{{ (e.report_bytes / 1024).toFixed(1) }} KB</td>
              <td style="font-size: 11px">{{ fmtTs(e.created_at) }}</td>
            </tr>
            <tr v-if="experiments.length === 0"><td colspan="6" class="muted" style="padding: 12px; text-align: center">暂无实验记录（差分矩阵跑完自动入库）</td></tr>
          </tbody>
        </table>
      </NTabPane>

      <NTabPane name="brute" tab="爆破作业">
        <table class="plain-table" style="margin-top: 8px">
          <thead>
            <tr><th style="width: 60px">#</th><th>案件</th><th style="width: 80px">引擎</th><th style="width: 90px">状态</th><th>命中</th><th style="width: 90px">空间</th><th>性能</th></tr>
          </thead>
          <tbody>
            <tr v-for="b in bruteJobs" :key="b.id">
              <td class="mono">{{ b.id }}</td>
              <td style="font-size: 12px">{{ b.case_name }}</td>
              <td><NTag size="tiny" :bordered="false">{{ b.engine }}</NTag></td>
              <td><span :class="['status-chip', b.status === 'hit' ? 'status-chip--pass' : 'status-chip--pending']">{{ b.status }}</span></td>
              <td>
                <b v-if="b.hit_value" class="mono" style="color: var(--st-pass)">{{ b.hit_value }}</b>
                <span v-else class="muted">—</span>
                <NTag v-if="b.hit_value && !b.self_test_passed" size="tiny" type="warning" :bordered="false">自测未过</NTag>
              </td>
              <td class="mono" style="font-size: 11px">{{ b.candidates_total.toLocaleString() }}</td>
              <td style="font-size: 11px; max-width: 200px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap">{{ b.perf_note }}</td>
            </tr>
            <tr v-if="bruteJobs.length === 0"><td colspan="7" class="muted" style="padding: 12px; text-align: center">暂无爆破作业（还原/爆破跑过自动入库）</td></tr>
          </tbody>
        </table>
      </NTabPane>
    </NTabs>
  </div>
</template>
