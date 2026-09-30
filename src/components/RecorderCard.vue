<script setup lang="ts">
import { onMounted, ref } from "vue";
import { NButton, useMessage } from "naive-ui";
import { DownloadOutline, TrashOutline, RefreshOutline } from "@vicons/ionicons5";
import { api, type RecordedStep } from "@/api";

/** Recorder v1（文档03§五）：操作 → Step（等价命令+参数+结果+耗时），可导出可重放 */
const steps = ref<RecordedStep[]>([]);
const loading = ref(false);
const message = useMessage();

async function refresh() {
  loading.value = true;
  try {
    steps.value = await api.recorderList();
  } catch (e) {
    message.error(String(e));
  } finally {
    loading.value = false;
  }
}

async function doExport(format: "ps1" | "sh" | "md" | "json") {
  try {
    const r = await api.recorderExport(format);
    message.success(`已导出 ${r.count} 步 → ${r.path}`);
  } catch (e) {
    message.error(String(e));
  }
}

async function doClear() {
  await api.recorderClear();
  await refresh();
}

onMounted(() => {
  refresh();
  // 安装/附加等长操作完成时会写入记录，轮询保持卡片新鲜
  setInterval(refresh, 5000);
});
defineExpose({ refresh });
</script>

<template>
  <div class="card info-card">
    <h3>
      操作记录（Recorder）
      <span class="muted" style="font-weight: 400">可视化操作自动生成 Step，可导出重放/入笔记</span>
    </h3>
    <div class="connect-row" style="margin-bottom: 10px">
      <NButton size="tiny" secondary :loading="loading" @click="refresh">
        <template #icon><RefreshOutline /></template>
        刷新
      </NButton>
      <NButton size="tiny" secondary :disabled="steps.length === 0" @click="doExport('md')">导出 md（带截图位）</NButton>
      <NButton size="tiny" secondary :disabled="steps.length === 0" @click="doExport('sh')">导出 sh</NButton>
      <NButton size="tiny" secondary :disabled="steps.length === 0" @click="doExport('ps1')">导出 ps1</NButton>
      <NButton size="tiny" secondary :disabled="steps.length === 0" @click="doExport('json')">导出 json</NButton>
      <NButton size="tiny" quaternary type="warning" :disabled="steps.length === 0" @click="doClear">
        <template #icon><TrashOutline /></template>
        清空
      </NButton>
      <span class="muted">共 {{ steps.length }} 步</span>
    </div>
    <div class="recorder-list">
      <table v-if="steps.length" class="plain-table">
        <thead>
          <tr><th style="width: 40px">#</th><th>操作</th><th>等价命令</th><th>结果</th><th style="width: 70px">耗时</th></tr>
        </thead>
        <tbody>
          <tr v-for="s in [...steps].reverse().slice(0, 20)" :key="s.seq">
            <td class="mono">{{ s.seq }}</td>
            <td>{{ s.action }}</td>
            <td class="mono" style="font-size: 11px; max-width: 340px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap">
              {{ s.command }}
            </td>
            <td style="font-size: 11px">{{ s.result }}</td>
            <td class="mono" style="font-size: 11px">{{ s.duration_ms }}ms</td>
          </tr>
        </tbody>
      </table>
      <div v-else class="muted" style="padding: 8px 0">
        暂无记录。连接、安装、附加等操作会自动记录。
        <DownloadOutline style="vertical-align: -2px" />
      </div>
    </div>
  </div>
</template>
