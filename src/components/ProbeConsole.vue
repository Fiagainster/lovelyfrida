<script setup lang="ts">
import { computed, onMounted, onUnmounted, reactive, ref } from "vue";
import { NButton, NInput, NInputNumber, NSelect, NSwitch, NTag, useMessage } from "naive-ui";
import { SearchOutline, AddOutline, TrashOutline, ArrowForwardOutline } from "@vicons/ionicons5";
import { useProbeStore } from "@/stores/probe";
import { useSessionStore } from "@/stores/session";
import type { ProbeStat } from "@/api";
import StatusLight from "@/components/StatusLight.vue";

/** 探针注入节点（M2）：探针工作台 / 探索器 / REPL 三 tab */
const probe = useProbeStore();
const session = useSessionStore();
const message = useMessage();

const tab = ref<"probes" | "explorer" | "repl">("probes");

// ---------- 探针表单 ----------
const form = reactive({
  clazz: "",
  method: "",
  maxLen: 128,
  captureRet: true,
  condition: "",
});
const needSession = computed(() => session.session?.phase !== "running");

async function onAddProbe() {
  if (!form.clazz.trim() || !form.method.trim()) {
    message.warning("类名和方法名都要填（构造函数方法名填 $init）");
    return;
  }
  try {
    const status = await probe.addProbe({
      clazz: form.clazz.trim(),
      method: form.method.trim(),
      maxLen: form.maxLen,
      captureRet: form.captureRet,
      condition: form.condition.trim() || undefined,
    });
    if (status === "waiting") {
      message.warning("类未加载，探针进入 waiting（会自动延迟重试，P-05）");
    } else {
      message.success("探针已挂载");
    }
  } catch (e) {
    message.error(String(e));
  }
}

// ---------- 探索器 ----------
const exMode = ref<"classes" | "methods">("classes");
const exQuery = ref("");
const exLoading = ref(false);
const exClassRows = ref<{ name: string }[]>([]);
const exMethodRows = ref<{ className: string; methodName: string; isStatic: boolean | null }[]>([]);
const exTotal = ref(0);
const selectedClass = ref<string | null>(null);
const classMethods = ref<{ name: string; signature: string; isStatic: boolean; isConstructor: boolean }[]>([]);
const classMethodsLoading = ref(false);

const modeOptions = [
  { label: "类搜索（包含匹配）", value: "classes" },
  { label: "方法搜索（*类!*方法 通配）", value: "methods" },
];

async function onSearch() {
  exLoading.value = true;
  try {
    if (exMode.value === "classes") {
      const r = await probe.rpc<{ total: number; rows: { name: string }[] }>("javaClasses", [
        { query: exQuery.value, limit: 100, offset: 0 },
      ]);
      exClassRows.value = r.rows;
      exTotal.value = r.total;
      exMethodRows.value = [];
    } else {
      const q = exQuery.value.includes("!")
        ? exQuery.value
        : `*${exQuery.value}*!*${exQuery.value}*`;
      const r = await probe.rpc<{ total: number; rows: { className: string; methodName: string; isStatic: boolean | null }[] }>(
        "javaSearchMethods",
        [{ query: q, limit: 200 }],
      );
      exMethodRows.value = r.rows;
      exTotal.value = r.total;
      exClassRows.value = [];
    }
  } catch (e) {
    message.error(String(e));
  } finally {
    exLoading.value = false;
  }
}

async function pickClass(name: string) {
  selectedClass.value = name;
  classMethodsLoading.value = true;
  try {
    const r = await probe.rpc<{ methods: typeof classMethods.value; error: string | null }>("javaMethods", [
      { className: name },
    ]);
    if (r.error) {
      message.warning(`getDeclaredMethods 失败：${r.error}（可能是加固壳，尝试切换 loader）`);
      classMethods.value = [];
    } else {
      classMethods.value = r.methods;
    }
  } finally {
    classMethodsLoading.value = false;
  }
}

function hookMethod(clazz: string, method: string) {
  form.clazz = clazz;
  form.method = method;
  tab.value = "probes";
  message.info(`已带入探针表单：${clazz}.${method}，点「挂探针」即生效`);
}

// ---------- REPL ----------
const replInput = ref("");
const replBusy = ref(false);
interface ReplEntry {
  code: string;
  ok: boolean;
  text: string;
}
const replHistory = ref<ReplEntry[]>([]);

async function onReplRun() {
  const code = replInput.value.trim();
  if (!code) return;
  replBusy.value = true;
  try {
    const r = await probe.rpc<{ value: { k: string; v: string }; blob_b64: string | null }>("replEval", [{ code }]);
    replHistory.value.unshift({ code, ok: r.value.k !== "err", text: `[${r.value.k}] ${r.value.v}` });
    replInput.value = "";
  } catch (e) {
    replHistory.value.unshift({ code, ok: false, text: String(e) });
  } finally {
    replBusy.value = false;
  }
}

// ---------- 探针状态轮询 ----------
let timer: number | null = null;
onMounted(() => {
  if (session.session?.phase === "running") void probe.refreshStats();
  timer = window.setInterval(() => {
    if (session.session?.phase === "running") void probe.refreshStats();
  }, 2500);
});
onUnmounted(() => {
  if (timer) window.clearInterval(timer);
});

function statusLight(s: ProbeStat["status"]) {
  return s === "active" ? "pass" : s === "waiting" ? "running" : "fail";
}
</script>

<template>
  <!-- 诊断卡片（先真因后处置，文档05） -->
  <div v-if="probe.diagnostics.length" class="diag-cards">
    <div v-for="d in probe.diagnostics" :key="d.id" class="diag-card">
      <div class="diag-card__title">◑ {{ d.title }} <NTag size="tiny" :bordered="false">{{ d.rule }}</NTag></div>
      <div class="diag-card__cause">真因：{{ d.cause }}</div>
      <div class="diag-card__fix">处置：{{ d.fix }}</div>
    </div>
  </div>

  <div v-if="needSession" class="placeholder-view" style="height: 300px">
    <span>需要先附加一个目标会话（设备连接节点 → 枚举进程 → 附加）</span>
    <span class="placeholder-view__milestone">附加后此节点提供 探针 / 探索器 / REPL</span>
  </div>

  <template v-else>
    <div class="probe-tabs">
      <div class="probe-tab" :class="{ 'probe-tab--active': tab === 'probes' }" @click="tab = 'probes'">探针工作台</div>
      <div class="probe-tab" :class="{ 'probe-tab--active': tab === 'explorer' }" @click="tab = 'explorer'">探索器</div>
      <div class="probe-tab" :class="{ 'probe-tab--active': tab === 'repl' }" @click="tab = 'repl'">REPL</div>
      <div style="flex: 1" />
      <NButton size="tiny" quaternary :loading="probe.statsLoading" @click="probe.refreshStats()">刷新状态</NButton>
    </div>

    <!-- ============ 探针工作台 ============ -->
    <div v-if="tab === 'probes'" class="card info-card">
      <h3>挂新探针（填表即挂钩，零行 JS）</h3>
      <div class="probe-form">
        <NInput v-model:value="form.clazz" size="small" placeholder="完整类名，如 com.example.Crypto（内嵌类用 Outer$Inner）" style="flex: 2" />
        <NInput v-model:value="form.method" size="small" placeholder="方法名（构造函数填 $init）" style="flex: 1" />
        <NInputNumber v-model:value="form.maxLen" size="small" :min="16" :max="4096" :show-button="false" style="width: 100px" title="参数截断长度（O-02）" />
        <div class="probe-form__ret" title="捕获返回值">
          <span>返回值</span>
          <NSwitch v-model:value="form.captureRet" size="small" />
        </div>
        <NButton class="btn-hero" size="small" :loading="probe.adding" @click="onAddProbe">
          <template #icon><AddOutline /></template>
          挂探针
        </NButton>
      </div>
      <NInput
        v-model:value="form.condition"
        size="small"
        placeholder="可选命中条件（js 表达式，如 arguments[0].length > 4；留空 = 全部命中）"
        style="margin-top: 8px"
      />

      <h3 style="margin-top: 18px">已挂探针（{{ probe.probes.length }}）</h3>
      <table class="plain-table">
        <thead>
          <tr><th>状态</th><th>类.方法</th><th>命中</th><th>错误</th><th /></tr>
        </thead>
        <tbody>
          <tr v-for="p in probe.probes" :key="p.id">
            <td><StatusLight :status="statusLight(p.status)" show-text /></td>
            <td class="mono" style="font-size: 12px">{{ p.clazz }}.<b>{{ p.method }}</b></td>
            <td class="mono">{{ p.hits }}</td>
            <td>
              <span v-if="p.errors" class="mono" style="color: var(--st-fail); font-size: 11px">{{ p.errors }}</span>
              <span v-else-if="p.lastError" :title="p.lastError" class="muted">含错误</span>
              <span v-else class="muted">—</span>
            </td>
            <td>
              <NButton size="tiny" quaternary type="warning" @click="probe.removeProbe(p.id).catch(() => {})">
                <template #icon><TrashOutline /></template>
                卸载
              </NButton>
            </td>
          </tr>
          <tr v-if="probe.probes.length === 0">
            <td colspan="5" class="muted">还没有探针。用下方探索器搜索目标，或直接填类名方法名。</td>
          </tr>
        </tbody>
      </table>
    </div>

    <!-- ============ 探索器 ============ -->
    <div v-else-if="tab === 'explorer'" class="card info-card">
      <h3>探索器（attach 后第一件事：找到目标）</h3>
      <div class="connect-row">
        <NSelect v-model:value="exMode" :options="modeOptions" size="small" style="width: 230px" />
        <NInput
          v-model:value="exQuery"
          size="small"
          placeholder="如 settings / *cipher* / *password*"
          style="width: 320px"
          clearable
          @keydown.enter="onSearch"
        />
        <NButton size="small" type="primary" secondary :loading="exLoading" @click="onSearch">
          <template #icon><SearchOutline /></template>
          搜索
        </NButton>
        <span v-if="exTotal" class="muted">{{ exTotal }} 条</span>
      </div>

      <!-- 类结果 -->
      <div v-if="exMode === 'classes' && exClassRows.length" class="explorer-results">
        <table class="plain-table">
          <thead><tr><th>类名</th><th style="width: 110px" /></tr></thead>
          <tbody>
            <tr v-for="r in exClassRows" :key="r.name" :class="{ 'row-selected': selectedClass === r.name }">
              <td class="mono" style="font-size: 12px">{{ r.name }}</td>
              <td>
                <NButton size="tiny" quaternary :loading="classMethodsLoading && selectedClass === r.name" @click="pickClass(r.name)">
                  查看方法 <ArrowForwardOutline style="vertical-align: -2px; margin-left: 2px" />
                </NButton>
              </td>
            </tr>
          </tbody>
        </table>
      </div>

      <!-- 方法结果 -->
      <div v-if="exMode === 'methods' && exMethodRows.length" class="explorer-results">
        <table class="plain-table">
          <thead><tr><th>类</th><th>方法</th><th>static</th><th style="width: 90px" /></tr></thead>
          <tbody>
            <tr v-for="(r, i) in exMethodRows" :key="i">
              <td class="mono" style="font-size: 12px">{{ r.className }}</td>
              <td><b>{{ r.methodName }}</b></td>
              <td>{{ r.isStatic === null ? "—" : r.isStatic ? "✔" : "✘" }}</td>
              <td>
                <NButton size="tiny" type="primary" quaternary @click="hookMethod(r.className, r.methodName)">挂探针</NButton>
              </td>
            </tr>
          </tbody>
        </table>
      </div>

      <!-- 选中类的方法清单 -->
      <div v-if="selectedClass" class="explorer-methods">
        <h3 style="font-size: 12px; color: var(--text-3)">SELECTED · {{ selectedClass }}（{{ classMethods.length }} 个成员）</h3>
        <table class="plain-table">
          <thead><tr><th>名称</th><th>签名</th><th style="width: 90px" /></tr></thead>
          <tbody>
            <tr v-for="(m, i) in classMethods" :key="i">
              <td class="mono" style="font-size: 12px"><b>{{ m.name }}</b>{{ m.isConstructor ? " ⓒ" : "" }}</td>
              <td class="mono" style="font-size: 11px; color: var(--text-2); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 420px">
                {{ m.signature }}
              </td>
              <td>
                <NButton size="tiny" type="primary" quaternary @click="hookMethod(selectedClass, m.name)">挂探针</NButton>
              </td>
            </tr>
            <tr v-if="classMethods.length === 0"><td colspan="3" class="muted">加载中或无成员</td></tr>
          </tbody>
        </table>
      </div>
    </div>

    <!-- ============ REPL ============ -->
    <div v-else class="card info-card">
      <h3>REPL（直接在目标进程里求值；Java/Process/Module 全局可用）</h3>
      <div class="connect-row">
        <NInput
          v-model:value="replInput"
          size="small"
          placeholder="如 Java.use('java.lang.System').getProperty('os.version')"
          style="flex: 1"
          :disabled="replBusy"
          @keydown.enter="onReplRun"
        />
        <NButton size="small" type="primary" :loading="replBusy" @click="onReplRun">执行</NButton>
      </div>
      <div class="repl-feed mono">
        <div v-for="(h, i) in replHistory" :key="i" class="repl-entry">
          <div class="repl-code">&gt; {{ h.code }}</div>
          <div :class="h.ok ? 'repl-ok' : 'repl-err'">{{ h.text }}</div>
        </div>
        <div v-if="replHistory.length === 0" class="muted" style="padding: 8px">输出在这里显示（支持多轮，let/const 自动提升为 var）</div>
      </div>
    </div>
  </template>
</template>

<style>
.probe-tabs {
  display: flex;
  gap: 6px;
  margin-bottom: 14px;
  align-items: center;
}
.probe-tab {
  padding: 6px 16px;
  border-radius: var(--radius-sm);
  cursor: pointer;
  font-size: 13px;
  color: var(--text-2);
  border: 1px solid var(--border-1);
  background: var(--bg-panel);
  transition: all var(--trans-fast);
}
.probe-tab:hover {
  color: var(--text-1);
}
.probe-tab--active {
  color: #fff;
  background: var(--grad-accent);
  border-color: transparent;
  font-weight: 600;
}
.probe-form {
  display: flex;
  gap: 8px;
  align-items: center;
  flex-wrap: wrap;
}
.probe-form__ret {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  color: var(--text-2);
}
.explorer-results {
  margin-top: 12px;
  max-height: 300px;
  overflow-y: auto;
}
.explorer-methods {
  margin-top: 16px;
}
.row-selected {
  background: var(--accent-soft);
}
.repl-feed {
  margin-top: 12px;
  max-height: 340px;
  overflow-y: auto;
  background: var(--bg-terminal);
  border-radius: var(--radius-sm);
  padding: 10px 12px;
  display: flex;
  flex-direction: column;
  gap: 8px;
  color: #c9d4e0;
  font-size: 12px;
}
.repl-code {
  color: #7dd3fc;
}
.repl-ok {
  color: #86efac;
  white-space: pre-wrap;
  word-break: break-all;
}
.repl-err {
  color: #f87171;
  white-space: pre-wrap;
}
.diag-cards {
  margin-bottom: 14px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.diag-card {
  border: 1px solid var(--st-warn);
  background: var(--st-warn-soft);
  border-radius: var(--radius-md);
  padding: 10px 14px;
  font-size: 12px;
}
.diag-card__title {
  font-weight: 650;
  color: var(--st-warn);
  display: flex;
  gap: 8px;
  align-items: center;
  margin-bottom: 4px;
}
.diag-card__cause {
  color: var(--text-1);
  margin-bottom: 2px;
}
.diag-card__fix {
  color: var(--text-2);
}
</style>
