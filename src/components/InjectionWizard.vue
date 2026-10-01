<script setup lang="ts">
import { reactive, ref } from "vue";
import { NButton, NInput, useMessage } from "naive-ui";
import { AddOutline, TrashOutline, FlashOutline } from "@vicons/ionicons5";
import { api, type InjectionFile, type InjectionReport } from "@/api";
import StatusLight from "@/components/StatusLight.vue";

/** 数据回灌向导（文档04-C 七步逐条状态灯 + 登录态预警） */
const message = useMessage();
const pkg = ref("com.example.app");
const running = ref(false);
const report = ref<InjectionReport | null>(null);

const files = reactive<InjectionFile[]>([
  { localPath: "", deviceDir: "/data/data/com.example.app/files", deviceName: "password.json" },
]);

function addFile() {
  files.push({ localPath: "", deviceDir: files[0]?.deviceDir ?? "/data/data/", deviceName: "" });
}
function removeFile(i: number) {
  files.splice(i, 1);
}

function applyProfile(pk: string, dir: string) {
  pkg.value = pk;
  if (files.length > 0) files[0].deviceDir = dir;
}

defineExpose({ applyProfile });

async function onRun() {
  const valid = files.filter((f) => f.localPath.trim() && f.deviceDir.trim() && f.deviceName.trim());
  if (!pkg.value.trim() || valid.length === 0) {
    message.warning("包名和至少一个完整文件行（本地路径/目标目录/文件名）都要填");
    return;
  }
  running.value = true;
  try {
    report.value = await api.injectionRun(pkg.value.trim(), valid);
    if (report.value.overall === "pass") message.success("回灌完成，七步全绿");
    else if (report.value.overall === "warn") message.warning("回灌完成但含警告（看步骤详情与登录态预警）");
    else message.error("回灌失败（看失败步骤的处置）");
  } catch (e) {
    message.error(String(e));
  } finally {
    running.value = false;
  }
}
</script>

<template>
  <div class="card info-card">
    <h3>回灌配置</h3>
    <div class="connect-row" style="margin-bottom: 10px">
      <NInput v-model:value="pkg" size="small" placeholder="目标包名，如 com.hidden.calculator" style="width: 320px" />
    </div>
    <table class="plain-table">
      <thead>
        <tr><th>本地文件路径</th><th>设备目标目录</th><th>目标文件名</th><th style="width: 50px" /></tr>
      </thead>
      <tbody>
        <tr v-for="(f, i) in files" :key="i">
          <td><NInput v-model:value="f.localPath" size="small" placeholder="D:\\evidence\\...\\password.json" /></td>
          <td><NInput v-model:value="f.deviceDir" size="small" placeholder="/data/data/<pkg>/files" /></td>
          <td><NInput v-model:value="f.deviceName" size="small" placeholder="password.json" /></td>
          <td>
            <NButton v-if="files.length > 1" size="tiny" quaternary type="warning" @click="removeFile(i)">
              <template #icon><TrashOutline /></template>
            </NButton>
          </td>
        </tr>
      </tbody>
    </table>
    <div class="connect-row" style="margin-top: 10px">
      <NButton size="small" quaternary @click="addFile">
        <template #icon><AddOutline /></template>
        加文件
      </NButton>
      <NButton class="btn-hero" size="small" :loading="running" @click="onRun">
        <template #icon><FlashOutline /></template>
        运行回灌（七步）
      </NButton>
    </div>
    <p class="muted" style="margin: 8px 0 0">
      七步：停应用（D-06）→ 空跑建档（D-07）→ 建目录 → 推送（中转+su cp，D-01/D-02）→ 改属主（uid 实测）→
      restorecon（Enforcing）→ md5 校验。灌前会扫描登录态键并预警（O-03）。
    </p>
  </div>

  <!-- 登录态预警 -->
  <div v-if="report?.login_state_warnings.length" class="card info-card" style="border-color: var(--st-warn)">
    <h3 style="color: var(--st-warn)">⚠ 登录态预警（O-03）</h3>
    <div v-for="(w, i) in report.login_state_warnings" :key="i" style="font-size: 12px; color: var(--text-1); margin-bottom: 4px">
      {{ w }}
    </div>
  </div>

  <!-- 七步报告 -->
  <div v-if="report" class="card info-card">
    <h3>
      回灌报告
      <span :class="['status-chip', report.overall === 'pass' ? 'status-chip--pass' : report.overall === 'warn' ? 'status-chip--warn' : 'status-chip--fail']">
        {{ report.overall === "pass" ? "七步全绿" : report.overall === "warn" ? "含警告" : "失败" }}
      </span>
    </h3>
    <div class="inj-steps">
      <div v-for="(s, i) in report.steps" :key="i" class="inj-step">
        <StatusLight
          :status="s.status === 'pass' ? 'pass' : s.status === 'warn' ? 'warn' : s.status === 'fail' ? 'fail' : 'skip'"
          :size="10"
        />
        <b style="min-width: 120px">{{ s.name }}</b>
        <div class="inj-step__ev">
          <div v-for="(e, j) in s.evidence" :key="j" class="mono" style="font-size: 11px; color: var(--text-2)">
            {{ e }}
          </div>
        </div>
      </div>
    </div>
  </div>
  <div v-else-if="!report" class="placeholder-view" style="height: 120px">
    <span class="placeholder-view__milestone">填写配置后运行，逐步证据会显示在这里</span>
  </div>
</template>

<style>
.inj-steps {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.inj-step {
  display: flex;
  gap: 10px;
  align-items: flex-start;
  font-size: 13px;
}
.inj-step__ev {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
</style>
