<script setup lang="ts">
import { computed, onMounted, reactive, ref } from "vue";
import { NButton, NInput, NTag, useMessage } from "naive-ui";
import {
  SaveOutline,
  TrashOutline,
  PlayOutline,
  CubeOutline,
  SearchOutline,
} from "@vicons/ionicons5";
import { api, type ScriptInfo } from "@/api";
import { useProbeStore } from "@/stores/probe";
import { useSessionStore } from "@/stores/session";

/** 深度观测 + 脚本库 + 实例检查器（能力包 A/B/C） */
const probe = useProbeStore();
const session = useSessionStore();
const message = useMessage();

const needSession = computed(() => session.session?.phase !== "running");

// ---------- 深度观测（能力包 A/C：dlopen / RegisterNatives / dumpDex / SSL） ----------
const deepBusy = ref<string | null>(null);
const deepLog = ref<{ ts: string; text: string }[]>([]);
const sslWatchId = ref<string | null>(null);

function deepLogPush(text: string) {
  deepLog.value.unshift({ ts: new Date().toLocaleTimeString("zh-CN", { hour12: false }), text });
  if (deepLog.value.length > 60) deepLog.value.pop();
}

async function deep(f: string, args: unknown[], label: string) {
  deepBusy.value = f;
  try {
    const r = await probe.rpc(f, args);
    deepLogPush(`${label} → ${JSON.stringify(r)}`);
    return r;
  } catch (e) {
    deepLogPush(`${label} ✖ ${String(e).slice(0, 120)}`);
    message.error(String(e));
    return null;
  } finally {
    deepBusy.value = null;
  }
}

async function onDumpDex() {
  const r = (await deep("dumpDex", [{ maxDex: 8 }], "dumpDex（内存 dex 搜索）")) as {
    found: unknown[];
    dumped: unknown[];
  } | null;
  if (r) message.info(`找到 ${r.found.length} 个 dex，成功落盘 ${r.dumped.length} 个（cases/dumps/）`);
}

async function onSsl() {
  if (sslWatchId.value) {
    message.info(`SSL 监控进行中（先看时间轴的 ssl_data 事件）`);
    return;
  }
  const id = "ssl-" + Date.now().toString(36);
  const r = (await deep("watchSsl", [{ id, maxBuf: 512 }], "SSL read/write 监控")) as {
    ok: boolean;
    hooks: string[];
    error: string | null;
  } | null;
  if (r?.ok) {
    sslWatchId.value = id;
    message.success(`SSL 缓冲捕获已挂载：${r.hooks.join("/")}（网络数据将实时进时间轴）`);
  }
}

// ---------- 脚本库（能力包 B） ----------
const scripts = ref<ScriptInfo[]>([]);
const scriptName = ref("");
const scriptContent = ref("");
const scriptRunning = ref(false);
const scriptResult = ref<string | null>(null);

async function refreshScripts() {
  scripts.value = await api.scriptList();
}

async function onOpenScript(name: string) {
  scriptName.value = name;
  scriptContent.value = await api.scriptRead(name);
}

async function onSaveScript() {
  if (!scriptName.value.trim()) {
    message.warning("先填脚本名");
    return;
  }
  await api.scriptSave(scriptName.value.trim(), scriptContent.value);
  message.success("已保存（cases/scripts/）");
  await refreshScripts();
}

async function onDeleteScript() {
  if (!scriptName.value.trim()) return;
  await api.scriptDelete(scriptName.value.trim());
  scriptName.value = "";
  scriptContent.value = "";
  await refreshScripts();
  message.success("已删除");
}

async function onRunScript() {
  const code = scriptContent.value;
  if (!code.trim()) return;
  scriptRunning.value = true;
  scriptResult.value = null;
  try {
    const r = await probe.rpc<{ value: { k: string; v: string } }>("replEval", [{ code }]);
    scriptResult.value = `[${r.value.k}] ${r.value.v.slice(0, 2000)}`;
    message.success("脚本执行完成（钩子已在目标内持久生效；console 输出进时间轴）");
  } catch (e) {
    scriptResult.value = String(e).slice(0, 500);
    message.error("脚本执行失败");
  } finally {
    scriptRunning.value = false;
  }
}

// ---------- 实例检查器（能力包 A） ----------
const inst = reactive({ className: "", limit: 20 });
const instLoading = ref(false);
const instCount = ref(0);
const instRows = ref<{ hashCode: number; toString: string }[]>([]);
const invokeRow = reactive({ hashCode: 0, methodName: "", arg: "" });
const invokeResult = ref<string | null>(null);

async function onChoose() {
  if (!inst.className.trim()) return;
  instLoading.value = true;
  try {
    const r = await probe.rpc<{ count: number; instances: { hashCode: number; toString: string }[] }>(
      "chooseDetailed",
      [{ className: inst.className.trim(), limit: inst.limit }],
    );
    instCount.value = r.count;
    instRows.value = r.instances;
  } catch (e) {
    message.error(String(e));
  } finally {
    instLoading.value = false;
  }
}

async function onInvoke() {
  if (!inst.className.trim() || !invokeRow.methodName.trim()) return;
  try {
    const args = invokeRow.arg.trim() ? invokeRow.arg.split("||").map((s) => s.trim()) : [];
    const r = await probe.rpc<{ ok: boolean; result: string; error: string | null }>("invokeInstance", [
      { className: inst.className.trim(), hashCode: invokeRow.hashCode, methodName: invokeRow.methodName.trim(), args },
    ]);
    invokeResult.value = r.ok ? r.result : `错误：${r.error}`;
  } catch (e) {
    invokeResult.value = String(e);
  }
}

onMounted(refreshScripts);
</script>

<template>
  <div v-if="needSession" class="placeholder-view" style="height: 300px">
    <span>需要先附加目标会话</span>
  </div>
  <template v-else>
    <!-- ============ 深度观测 ============ -->
    <div class="card info-card">
      <h3>深度观测（加固壳 / native / 网络 / 内存）</h3>
      <div class="connect-row" style="flex-wrap: wrap">
        <NButton size="small" secondary :loading="deepBusy === 'watchDlopen'" @click="deep('watchDlopen', [], 'dlopen 监控')">
          <CubeOutline /> dlopen 监控（加固 so 加载）
        </NButton>
        <NButton size="small" secondary :loading="deepBusy === 'watchRegisterNatives'" @click="deep('watchRegisterNatives', [], 'RegisterNatives 捕获')">
          <CubeOutline /> RegisterNatives 捕获（JNI 动态注册）
        </NButton>
        <NButton size="small" secondary :loading="deepBusy === 'dumpDex'" @click="onDumpDex">
          <CubeOutline /> dumpDex（内存 dex 搜索落盘）
        </NButton>
        <NButton size="small" secondary :loading="deepBusy === 'watchSsl'" @click="onSsl">
          <CubeOutline /> SSL 缓冲捕获（网络观测）
        </NButton>
      </div>
      <div v-if="deepLog.length" class="repl-feed mono" style="margin-top: 10px; max-height: 200px">
        <div v-for="(l, i) in deepLog" :key="i" :class="l.text.includes('✖') ? 'repl-err' : 'repl-ok'" style="font-size: 11px">
          {{ l.ts }} {{ l.text.slice(0, 220) }}
        </div>
      </div>
      <p class="muted" style="margin: 8px 0 0; font-size: 11px">
        dlopen/RegisterNatives/ssl_data 事件实时进时间轴；dumpDex 落 cases/dumps/（只读观测，不解包不修改）。
      </p>
    </div>

    <!-- ============ 脚本库 ============ -->
    <div class="card info-card">
      <h3>脚本库（任意 frida 脚本经 GUI 运行；钩子在目标内持久）</h3>
      <div class="connect-row" style="margin-bottom: 8px; flex-wrap: wrap">
        <NInput v-model:value="scriptName" size="small" placeholder="脚本名" style="width: 180px" />
        <NButton size="tiny" secondary @click="onSaveScript"><SaveOutline /> 保存</NButton>
        <NButton size="tiny" quaternary type="warning" @click="onDeleteScript"><TrashOutline /> 删除</NButton>
        <NTag v-for="s in scripts" :key="s.name" size="small" :bordered="false" style="cursor: pointer" @click="onOpenScript(s.name)">
          {{ s.name }}.js
        </NTag>
      </div>
      <NInput
        v-model:value="scriptContent"
        type="textarea"
        :rows="10"
        class="mono"
        placeholder="// 粘贴任意 frida 脚本：Java.perform / Interceptor.attach / rpc.exports 均可"
      />
      <div class="connect-row" style="margin-top: 8px">
        <NButton class="btn-hero" size="small" :loading="scriptRunning" @click="onRunScript">
          <template #icon><PlayOutline /></template>
          运行脚本
        </NButton>
        <span v-if="scriptResult" class="mono" style="font-size: 11px; color: var(--text-2)">{{ scriptResult.slice(0, 160) }}</span>
      </div>
    </div>

    <!-- ============ 实例检查器 ============ -->
    <div class="card info-card">
      <h3>实例检查器（Java.choose + 定向调用）</h3>
      <div class="connect-row">
        <NInput v-model:value="inst.className" size="small" placeholder="类名，如 com.example.app.model.User" style="width: 340px" />
        <NButton size="small" secondary :loading="instLoading" @click="onChoose">
          <template #icon><SearchOutline /></template>
          搜实例
        </NButton>
        <span v-if="instCount" class="muted">堆上 {{ instCount }} 个实例</span>
      </div>
      <table v-if="instRows.length" class="plain-table" style="margin-top: 8px">
        <thead><tr><th style="width: 110px">hashCode</th><th>toString</th><th>定向调用</th></tr></thead>
        <tbody>
          <tr v-for="r in instRows" :key="r.hashCode">
            <td class="mono" style="font-size: 11px">{{ r.hashCode }}</td>
            <td style="font-size: 11px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 380px">{{ r.toString }}</td>
            <td>
              <NButton size="tiny" quaternary @click="invokeRow.hashCode = r.hashCode">选中</NButton>
            </td>
          </tr>
        </tbody>
      </table>
      <div v-if="invokeRow.hashCode" class="connect-row" style="margin-top: 8px">
        <NTag size="small" :bordered="false">实例 {{ invokeRow.hashCode }}</NTag>
        <NInput v-model:value="invokeRow.methodName" size="small" placeholder="方法名（无参）" style="width: 160px" />
        <NInput v-model:value="invokeRow.arg" size="small" placeholder="参数（|| 分隔，按字符串传入）" style="flex: 1" />
        <NButton size="small" secondary @click="onInvoke">调用</NButton>
      </div>
      <div v-if="invokeResult" class="repl-ok mono" style="font-size: 12px; margin-top: 6px">→ {{ invokeResult.slice(0, 300) }}</div>
    </div>
  </template>
</template>
