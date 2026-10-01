<script setup lang="ts">
import { computed, onMounted, reactive, ref } from "vue";
import { NButton, NInput, NSelect, NTag, useMessage } from "naive-ui";
import { AddOutline, ArchiveOutline, DownloadOutline, TrashOutline } from "@vicons/ionicons5";
import { api, type Finding } from "@/api";
import StatusLight from "@/components/StatusLight.vue";

/** 档案台账（文档04-H / M5）：发现登记 + 置信度约束（high=双证据）+ 导出 + 案卷包 */
const message = useMessage();
const caseName = ref("默认案件");
const findings = ref<Finding[]>([]);
const loading = ref(false);

const form = reactive({
  questionId: "",
  question: "",
  answer: "",
  confidence: "medium",
  evMath: false,
  evDevice: false,
  evNote: "",
  source: "",
  screenshotSlot: "",
});

const confOptions = [
  { label: "high（数学自证 + 真机复现 双证据）", value: "high" },
  { label: "medium（单证据）", value: "medium" },
  { label: "low（推断，待验证）", value: "low" },
];

const highBlockHint = computed(
  () => form.confidence === "high" && !(form.evMath && form.evDevice),
);

async function refresh() {
  loading.value = true;
  try {
    findings.value = await api.ledgerList(caseName.value.trim() || "default");
  } catch (e) {
    message.error(String(e));
  } finally {
    loading.value = false;
  }
}

async function onAdd() {
  const evidence: { kind: string; note: string }[] = [];
  if (form.evMath) evidence.push({ kind: "math", note: form.evNote || "数学自证" });
  if (form.evDevice) evidence.push({ kind: "device", note: form.evNote || "真机复现" });
  try {
    await api.ledgerAdd({
      caseName: caseName.value.trim() || "default",
      questionId: form.questionId.trim() || `Q${findings.value.length + 1}`,
      question: form.question.trim(),
      answer: form.answer.trim(),
      confidence: form.confidence,
      evidence,
      source: form.source.trim(),
      screenshotSlot: form.screenshotSlot.trim(),
    });
    message.success("发现已登记");
    form.questionId = "";
    form.question = "";
    form.answer = "";
    form.evMath = false;
    form.evDevice = false;
    form.evNote = "";
    await refresh();
  } catch (e) {
    message.error(String(e));
  }
}

async function onDelete(id: number) {
  await api.ledgerDelete(id);
  await refresh();
}

async function exportMd() {
  try {
    const p = await api.ledgerExportMd(caseName.value.trim() || "default");
    message.success(`已导出 → ${p}`);
  } catch (e) {
    message.error(String(e));
  }
}

async function exportBundle() {
  try {
    const p = await api.ledgerExportBundle(caseName.value.trim() || "default");
    message.success(`案卷包已生成 → ${p}`);
  } catch (e) {
    message.error(String(e));
  }
}

onMounted(refresh);
</script>

<template>
  <!-- 登记表单 -->
  <div class="card info-card">
    <h3>登记发现（Finding）</h3>
    <div class="connect-row" style="margin-bottom: 8px">
      <NInput v-model:value="caseName" size="small" placeholder="案名" style="width: 200px" @blur="refresh" />
      <NInput v-model:value="form.questionId" size="small" placeholder="编号，如 Q1 / Finding-01" style="width: 130px" />
      <NInput v-model:value="form.question" size="small" placeholder="问题，如：用户密码是什么" style="flex: 1" />
    </div>
    <div class="connect-row" style="margin-bottom: 8px">
      <NInput v-model:value="form.answer" size="small" placeholder="分析结论或密码" style="flex: 1" />
      <NSelect v-model:value="form.confidence" :options="confOptions" size="small" style="width: 300px" />
    </div>
    <div class="connect-row" style="margin-bottom: 8px">
      <span class="muted">证据：</span>
      <NTag
        size="small"
        :type="form.evMath ? 'success' : 'default'"
        :bordered="false"
        style="cursor: pointer"
        @click="form.evMath = !form.evMath"
      >
        <StatusLight :status="form.evMath ? 'pass' : 'pending'" :size="8" /> math 数学自证
      </NTag>
      <NTag
        size="small"
        :type="form.evDevice ? 'success' : 'default'"
        :bordered="false"
        style="cursor: pointer"
        @click="form.evDevice = !form.evDevice"
      >
        <StatusLight :status="form.evDevice ? 'pass' : 'pending'" :size="8" /> device 真机复现
      </NTag>
      <NInput v-model:value="form.evNote" size="small" placeholder="证据说明（如：重算逐字节一致）" style="flex: 1" />
    </div>
    <div class="connect-row" style="margin-bottom: 8px">
      <NInput v-model:value="form.source" size="small" placeholder="出处（如：时间轴 / 回灌向导 / 探索器）" style="flex: 1" />
      <NInput v-model:value="form.screenshotSlot" size="small" placeholder="截图位编号 N-x" style="width: 150px" />
    </div>
    <div class="connect-row">
      <NButton class="btn-hero" size="small" @click="onAdd">
        <template #icon><AddOutline /></template>
        登记
      </NButton>
      <span v-if="highBlockHint" class="muted" style="color: var(--st-warn)">
        high 需要 math + device 双证据同时勾选（原则5，数据库约束会拒绝）
      </span>
    </div>
  </div>

  <!-- 台账表 -->
  <div class="card info-card">
    <h3>
      发现台账（{{ caseName }}）
      <NTag size="small" :bordered="false">{{ findings.length }} 条</NTag>
    </h3>
    <div class="connect-row" style="margin-bottom: 8px">
      <NButton size="small" secondary :loading="loading" @click="refresh">刷新</NButton>
      <NButton size="small" secondary :disabled="findings.length === 0" @click="exportMd">
        <template #icon><DownloadOutline /></template>
        导出 Markdown（带截图位）
      </NButton>
      <NButton size="small" secondary :disabled="findings.length === 0" @click="exportBundle">
        <template #icon><ArchiveOutline /></template>
        生成案卷包（U10）
      </NButton>
    </div>
    <table class="plain-table">
      <thead>
        <tr><th style="width: 70px">题号</th><th>问题</th><th>答案</th><th style="width: 80px">置信</th><th>证据</th><th style="width: 50px" /></tr>
      </thead>
      <tbody>
        <tr v-for="f in findings" :key="f.id">
          <td class="mono" style="font-size: 11px">{{ f.question_id }}</td>
          <td style="font-size: 12px">{{ f.question }}</td>
          <td><b class="mono" style="font-size: 12px; color: var(--st-pass)">{{ f.answer }}</b></td>
          <td>
            <span :class="['status-chip', f.confidence === 'high' ? 'status-chip--pass' : f.confidence === 'medium' ? 'status-chip--warn' : 'status-chip--pending']">
              {{ f.confidence }}
            </span>
          </td>
          <td style="font-size: 11px">
            <NTag v-for="(e, i) in f.evidence" :key="i" size="tiny" :bordered="false" :type="e.kind === 'math' ? 'info' : 'success'" style="margin-right: 4px">
              {{ e.kind }}
            </NTag>
          </td>
          <td>
            <NButton size="tiny" quaternary type="warning" @click="onDelete(f.id)">
              <template #icon><TrashOutline /></template>
            </NButton>
          </td>
        </tr>
        <tr v-if="findings.length === 0">
          <td colspan="6" class="muted">暂无发现。上面的表单登记后，可导出 Obsidian 友好的 Markdown 或生成案卷包交接。</td>
        </tr>
      </tbody>
    </table>
  </div>
</template>
