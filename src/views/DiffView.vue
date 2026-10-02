<script setup lang="ts">
import { computed, reactive, ref } from "vue";
import { NButton, NInput, NInputNumber, NSelect, useMessage } from "naive-ui";
import { AddOutline, PlayOutline, TrashOutline } from "@vicons/ionicons5";
import { api, type ExperimentReport, type ExperimentTemplate } from "@/api";
import { useProbeStore } from "@/stores/probe";
import { useSessionStore } from "@/stores/session";
import { useCaseStore } from "@/stores/case";

/** 差分视图（文档03 / 04-E / U3）：受控实验台 + 差分矩阵（变异度排序、结论=候选） */
const probe = useProbeStore();
const session = useSessionStore();
const message = useMessage();

const cfg = reactive({
  package: "",
  deviceFileDir: "/data/data/com.example.app/files",
  deviceFileName: "password.json",
  probeId: "",
  waitS: 8,
});
const selectedProbe = computed(() => probe.probes.find((p) => p.id === cfg.probeId));
const templates = reactive<ExperimentTemplate[]>([
  { name: "组1", content: "" },
  { name: "组2", content: "" },
]);
const running = ref(false);
const report = ref<ExperimentReport | null>(null);

const probeOptions = computed(() =>
  probe.probes.map((p) => ({
    label: `${p.clazz}.${p.method}（${p.status}）`,
    value: p.id,
  })),
);

const needSession = computed(() => session.session?.phase !== "running");

function addTemplate() {
  templates.push({ name: `组${templates.length + 1}`, content: "" });
}

async function onRun() {
  if (!cfg.package.trim() || !cfg.deviceFileDir.trim() || !cfg.probeId) {
    message.warning("包名、目标文件目录、观测探针都要填（探针先在探针节点挂好）");
    return;
  }
  running.value = true;
  try {
    if (!selectedProbe.value) {
      message.warning("先选择观测探针");
      return;
    }
    report.value = await api.experimentRun({
      package: cfg.package.trim(),
      deviceFileDir: cfg.deviceFileDir.trim(),
      deviceFileName: cfg.deviceFileName.trim(),
      probe: {
        clazz: selectedProbe.value.clazz,
        method: selectedProbe.value.method,
        maxLen: 128,
        captureRet: true,
      },
      waitS: cfg.waitS,
      templates: templates.filter((t) => t.name.trim()),
    }, useCaseStore().apiCaseName());
    message.success(`实验完成：${report.value.observations.length} 组观测`);
  } catch (e) {
    message.error(String(e));
  } finally {
    running.value = false;
  }
}

// ---------- 差分矩阵（行=组，列=观测通道；变异度=该列不同值数） ----------
interface MatrixCol {
  label: string;
  values: (string | null)[];
  variance: number;
}

const matrix = computed<MatrixCol[]>(() => {
  if (!report.value) return [];
  const obs = report.value.observations;
  if (obs.length === 0) return [];
  const maxArgs = Math.max(...obs.map((o) => o.args?.length ?? 0));
  const cols: MatrixCol[] = [];
  for (let i = 0; i < maxArgs; i++) {
    const values = obs.map((o) => o.args?.[i]?.v ?? null);
    const distinct = new Set(values.filter((v) => v !== null)).size;
    cols.push({ label: `arg${i}`, values, variance: distinct });
  }
  if (obs.some((o) => o.ret)) {
    const values = obs.map((o) => o.ret?.v ?? null);
    const distinct = new Set(values.filter((v) => v !== null)).size;
    cols.push({ label: "返回值", values, variance: distinct });
  }
  cols.push({
    label: "命中次数",
    values: obs.map((o) => String(o.hits)),
    variance: new Set(obs.map((o) => o.hits)).size,
  });
  cols.sort((a, b) => b.variance - a.variance); // 变异度排序（文档03）
  return cols;
});

const candidates = computed(() => {
  const varying = matrix.value.filter((c) => c.variance > 1);
  if (varying.length === 0) return "尚无变异列——增加/修改输入组，或检查观测探针是否命中";
  return `候选结论：${varying.map((c) => `「${c.label}」随输入变化（${c.variance} 种取值）`).join("；")}。追加输入组可否证（U3）。`;
});
</script>

<template>
  <!-- 实验配置 -->
  <div class="card info-card">
    <h3>受控实验（变量 = 设备文件内容；观测通道 = 探针命中）</h3>
    <div v-if="needSession" class="muted" style="margin-bottom: 8px">
      ⚠ 先附加目标会话（设备连接节点），再挂观测探针（探针注入节点）。
    </div>
    <div class="connect-row" style="margin-bottom: 8px">
      <NInput v-model:value="cfg.package" size="small" placeholder="目标包名" style="width: 260px" />
      <NInput v-model:value="cfg.deviceFileDir" size="small" placeholder="/data/data/<pkg>/files" style="width: 300px" />
      <NInput v-model:value="cfg.deviceFileName" size="small" placeholder="password.json" style="width: 160px" />
    </div>
    <div class="connect-row" style="margin-bottom: 8px">
      <NSelect
        v-model:value="cfg.probeId"
        :options="probeOptions"
        size="small"
        placeholder="选择观测探针（探针注入节点挂好后出现在这里）"
        style="width: 320px"
      />
      <span class="muted">等待窗口</span>
      <NInputNumber v-model:value="cfg.waitS" size="small" :min="2" :max="120" :show-button="false" style="width: 90px" />
      <span class="muted">s</span>
    </div>

    <table class="plain-table">
      <thead>
        <tr><th style="width: 90px">组名</th><th>文件内容（变量）</th><th style="width: 50px" /></tr>
      </thead>
      <tbody>
        <tr v-for="(t, i) in templates" :key="i">
          <td><NInput v-model:value="t.name" size="small" /></td>
          <td>
            <NInput
              v-model:value="t.content"
              type="textarea"
              size="small"
              :rows="1"
              placeholder="如 PABCDEFGHIJKLMNO（16 字节）——实验台会自动做快照与恢复"
            />
          </td>
          <td>
            <NButton v-if="templates.length > 2" size="tiny" quaternary type="warning" @click="templates.splice(i, 1)">
              <template #icon><TrashOutline /></template>
            </NButton>
          </td>
        </tr>
      </tbody>
    </table>
    <div class="connect-row" style="margin-top: 8px">
      <NButton size="small" quaternary @click="addTemplate">
        <template #icon><AddOutline /></template>
        加输入组
      </NButton>
      <NButton class="btn-hero" size="small" :loading="running" :disabled="needSession" @click="onRun">
        <template #icon><PlayOutline /></template>
        运行实验（{{ templates.length }} 组）
      </NButton>
    </div>
    <p class="muted" style="margin: 8px 0 0">
      每组：写入文件 → 停应用 → 重启 → 采集 {{ cfg.waitS }}s 内目标探针的命中。结论是「候选」不是「判决」（R5）；
      追加输入组可以否证刚得出的结论（U3）。
    </p>
  </div>

  <!-- 差分矩阵 -->
  <div v-if="report" class="card info-card">
    <h3>
      差分矩阵
      <NTag size="small" :bordered="false">exp-{{ report.experiment_id }}</NTag>
    </h3>
    <table class="plain-table diff-matrix">
      <thead>
        <tr>
          <th>输入组</th>
          <th v-for="c in matrix" :key="c.label" :class="{ 'diff-col': c.variance > 1 }">
            {{ c.label }}
            <span class="muted" style="font-size: 10px">变异 {{ c.variance }}</span>
          </th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="(o, i) in report.observations" :key="o.group + i">
          <td><b>{{ o.group }}</b><br /><span class="muted" style="font-size: 10px">{{ o.wall }}</span></td>
          <td v-for="c in matrix" :key="c.label" :class="{ 'diff-cell': c.variance > 1 }">
            {{ c.values[i] ?? "—" }}
          </td>
        </tr>
      </tbody>
    </table>
    <div class="diff-conclusion">
      <b>结论候选：</b>{{ candidates }}
    </div>
    <div v-if="report.errors.length" style="margin-top: 8px">
      <div v-for="(e, i) in report.errors" :key="i" class="mono" style="font-size: 11px; color: var(--st-fail)">{{ e }}</div>
    </div>
  </div>
  <div v-else class="placeholder-view" style="height: 140px">
    <span class="placeholder-view__milestone">运行实验后，差分矩阵在这里出现（含变异度排序与被支持的假说）</span>
  </div>
</template>



<style>
.diff-matrix .diff-col {
  color: var(--accent);
  font-weight: 650;
}
.diff-matrix .diff-cell {
  background: var(--accent-soft);
  font-weight: 600;
}
.diff-conclusion {
  margin-top: 10px;
  padding: 8px 12px;
  background: var(--bg-elevated);
  border-radius: var(--radius-sm);
  font-size: 12px;
  color: var(--text-1);
}
</style>
