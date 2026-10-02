<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { NButton, NInput, NTag, useMessage } from "naive-ui";
import {
  FlashOutline,
  RefreshOutline,
  SearchOutline,
  PlayCircleOutline,
  StopCircleOutline,
} from "@vicons/ionicons5";
import { useSessionStore } from "@/stores/session";
import { usePipelineStore } from "@/stores/pipeline";
import StatusLight from "@/components/StatusLight.vue";

/** 会话控制台（文档04-B）：frida 环境 + 进程分组列表 + 附加链路状态机 */
const session = useSessionStore();
const pipeline = usePipelineStore();
const message = useMessage();

const tab = ref<"user" | "system">("user");

const overall = computed(() => session.serverStatus?.overall ?? "pending");

const MATRIX_HINT = computed(() => {
  const m = session.serverStatus?.matrix ?? [];
  return m.length ? m.join(" / ") : "空（放入 bin\\frida-server\\<版本>\\android-<abi>\\frida-server）";
});

function versionMatch(): boolean {
  const s = session.serverStatus;
  return !!s?.client_version && s.client_version === s.device_server_version;
}

async function doAttach(p: { pid: number; name: string }) {
  try {
    await session.attach(p.pid);
    const snap = session.session;
    if (snap?.phase === "running") {
      message.success(`已附加 ${p.name}（session#${snap.session_id}），hello 握手成功`);
      pipeline.nodes[1].status = "pass";
      pipeline.nodes[1].summary = `会话 ${p.name}`;
    } else {
      message.warning("附加链路未到达 RUNNING（查看状态步进器证据）");
      pipeline.nodes[1].status = "fail";
    }
  } catch (e) {
    message.error(String(e));
    pipeline.nodes[1].status = "fail";
  }
}

async function doDetach() {
  await session.detachSession();
  message.info("已分离");
}

onMounted(() => {
  // 浏览器预览不发命令（isTauri 守卫在 api 层统一抛错，这里静默）
  if (session.serverStatus === null) {
    void session.refreshServerStatus().catch(() => {});
  }
  void session.refreshSession().catch(() => {});
});
</script>

<template>
  <!-- ============ Frida 环境 ============ -->
  <div class="card info-card">
    <h3>
      Frida 环境
      <span :class="['status-chip', `status-chip--${overall}`]">
        {{ overall === "pass" ? "三处一致" : overall === "warn" ? "部分就绪" : "异常" }}
      </span>
    </h3>
    <table class="plain-table">
      <tbody>
        <tr>
          <td style="width: 150px">① 本机客户端（通道B）</td>
          <td class="mono">
            {{ session.serverStatus?.client_version ?? "—" }}
            <span v-if="session.serverStatus?.client_error" class="muted">
              （{{ session.serverStatus.client_error }}）
            </span>
          </td>
          <td style="width: 90px">
            <StatusLight :status="session.serverStatus?.client_version ? 'pass' : 'fail'" :size="9" />
          </td>
        </tr>
        <tr>
          <td>② 设备端 frida-server</td>
          <td class="mono">
            <template v-if="session.serverStatus?.device_serial">
              {{ session.serverStatus.device_server_version ?? "未知版本" }}
              <span class="muted">（{{ session.serverStatus.device_server_present ? "已推送" : "未推送" }}）</span>
            </template>
            <span v-else class="muted">无设备</span>
          </td>
          <td>
            <StatusLight
              :status="versionMatch() ? 'pass' : session.serverStatus?.device_serial ? 'warn' : 'skip'"
              :size="9"
            />
          </td>
        </tr>
        <tr>
          <td>③ 运行状态（:{{ session.serverStatus?.port ?? 27042 }}）</td>
          <td class="mono">
            {{ session.serverStatus?.server_running ? "实测监听中" : "未监听" }}
          </td>
          <td>
            <StatusLight :status="session.serverStatus?.server_running ? 'pass' : 'warn'" :size="9" />
          </td>
        </tr>
        <tr>
          <td>④ adb forward</td>
          <td class="mono">{{ session.serverStatus?.forward_established ? "已建立" : "未建立" }}</td>
          <td>
            <StatusLight :status="session.serverStatus?.forward_established ? 'pass' : 'warn'" :size="9" />
          </td>
        </tr>
        <tr>
          <td>⑤ 版本矩阵</td>
          <td class="mono" style="font-size: 11px">{{ MATRIX_HINT }}</td>
          <td>
            <StatusLight :status="session.serverStatus?.matrix?.length ? 'pass' : 'warn'" :size="9" />
          </td>
        </tr>
      </tbody>
    </table>
    <div class="connect-row" style="margin-top: 12px">
      <NButton
        class="btn-hero"
        size="small"
        :loading="session.installLoading"
        :disabled="!session.serverStatus?.device_serial"
        title="清残留 → 推送匹配版 → chmod → 启动 → 实测监听（幂等）"
        @click="session.installServer().catch((e) => message.error(String(e)))"
      >
        <template #icon><FlashOutline /></template>
        安装并启动（推送匹配版）
      </NButton>
      <NButton
        size="small"
        secondary
        :disabled="!session.serverStatus?.device_serial"
        @click="session.setupForward().catch((e) => message.error(String(e)))"
      >
        仅建立 forward
      </NButton>
      <NButton size="small" quaternary @click="session.refreshServerStatus()">
        <template #icon><RefreshOutline /></template>
        刷新
      </NButton>
    </div>
    <!-- 安装逐步报告 -->
    <div v-if="session.installSteps" class="install-steps">
      <div v-for="(s, i) in session.installSteps" :key="i" class="install-step">
        <StatusLight :status="s.status === 'pass' ? 'pass' : s.status === 'warn' ? 'warn' : 'fail'" :size="9" />
        <b>{{ s.name }}</b>
        <span class="install-step__evidence mono">{{ s.evidence.join(" ｜ ") }}</span>
      </div>
    </div>
  </div>

  <!-- ============ 附加链路状态机 ============ -->
  <div class="card info-card">
    <h3>
      附加会话（通道{{ session.session?.channel ?? "B" }}{{ session.session?.channel === "C" ? " · CLI 兜底，仅观测" : "" }}）
      <span v-if="session.session?.phase === 'running'" class="status-chip status-chip--pass">
        {{ session.session.session_id != null ? `session#${session.session.session_id}` : "已附加" }} · {{ session.session.target }}
      </span>
      <span v-else-if="session.session?.phase === 'failed'" class="status-chip status-chip--fail">
        链路失败（保留现场，可单步重试）
      </span>
    </h3>
    <div class="phase-stepper">
      <template v-for="(p, i) in session.PHASES" :key="p.key">
        <div
          class="phase-node"
          :class="{
            'phase-node--done': session.phaseIndex > i,
            'phase-node--active': session.phaseIndex === i,
            'phase-node--failed': session.phaseIndex === -2 && i === 0,
          }"
        >
          <StatusLight
            :status="session.phaseIndex > i ? 'pass' : session.phaseIndex === i ? 'running' : session.phaseIndex === -2 && i === 0 ? 'fail' : 'pending'"
            :size="10"
          />
          <span>{{ p.label }}</span>
        </div>
        <div v-if="i < session.PHASES.length - 1" class="phase-arrow">→</div>
      </template>
      <div style="flex: 1" />
      <NButton
        v-if="session.session?.phase === 'running' || session.session?.session_id"
        size="tiny"
        type="warning"
        secondary
        @click="doDetach"
      >
        <template #icon><StopCircleOutline /></template>
        分离
      </NButton>
    </div>
    <!-- hello 证据 -->
    <div v-if="session.session?.hello" class="hello-box mono">
      hello：frida {{ session.session.hello.frida }} · pid {{ session.session.hello.pid }} ·
      {{ session.session.hello.platform }}/{{ session.session.hello.arch }} · Java {{ session.session.hello.java ?? "不可用" }}
    </div>
    <!-- 最近消息 -->
    <div v-if="session.messages.length" class="msg-feed mono">
      <div v-for="(m, i) in session.messages.slice(0, 8)" :key="i" class="msg-row">
        <span class="msg-ts">{{ m.ts }}</span>
        <span :class="['msg-kind', `msg-kind--${m.kind}`]">{{ m.kind }}</span>
        <span class="msg-text">{{ m.text.slice(0, 160) }}</span>
      </div>
    </div>
  </div>

  <!-- ============ 进程/应用列表 ============ -->
  <div class="card info-card">
    <h3>目标进程（attach 后即可挂探针/跑脚本）</h3>
    <div class="connect-row" style="margin-bottom: 10px">
      <NButton size="small" type="primary" secondary :loading="session.procsLoading" @click="session.refreshProcesses()">
        <template #icon><RefreshOutline /></template>
        枚举进程
      </NButton>
      <NInput
        v-model:value="session.procsFilter"
        size="small"
        placeholder="按包名/pid 过滤，如 calculator"
        style="width: 260px"
        clearable
      >
        <template #prefix><SearchOutline /></template>
      </NInput>
      <NTag size="small" :bordered="false" :type="tab === 'user' ? 'info' : 'default'" @click="tab = 'user'" style="cursor: pointer">
        用户应用
      </NTag>
      <NTag size="small" :bordered="false" :type="tab === 'system' ? 'warning' : 'default'" @click="tab = 'system'" style="cursor: pointer">
        系统进程
      </NTag>
      <span class="muted">共 {{ session.processes.length }} 条</span>
    </div>
    <div class="proc-list">
      <table class="plain-table">
        <thead>
          <tr><th style="width: 90px">pid</th><th>名称 / 包名</th><th style="width: 90px">状态</th><th style="width: 80px" /></tr>
        </thead>
        <tbody>
          <tr v-for="p in session.filteredProcesses.filter((x) => x.group === tab).slice(0, 60)" :key="p.pid">
            <td class="mono">{{ p.pid }}</td>
            <td>{{ p.name }}</td>
            <td>
              <span :class="['status-chip', p.running ? 'status-chip--pass' : 'status-chip--pending']">
                {{ p.running ? "运行中" : "未运行" }}
              </span>
            </td>
            <td>
              <NButton
                size="tiny"
                type="primary"
                quaternary
                :loading="session.attachLoading"
                title="附加并注入 core agent"
                @click="doAttach(p)"
              >
                <template #icon><PlayCircleOutline /></template>
                附加
              </NButton>
            </td>
          </tr>
          <tr v-if="session.processes.length === 0">
            <td colspan="4" class="muted">
              尚未枚举。先确保 frida-server 在运行（上方安装并启动），再点「枚举进程」。
            </td>
          </tr>
        </tbody>
      </table>
    </div>
  </div>
</template>

<style>
.install-steps {
  margin-top: 10px;
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.install-step {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 12px;
}
.install-step__evidence {
  color: var(--text-3);
  font-size: 11px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.phase-stepper {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
  padding: 6px 0;
}
.phase-node {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  color: var(--text-3);
  padding: 4px 10px;
  border-radius: 999px;
  border: 1px solid var(--border-1);
  background: var(--bg-elevated);
}
.phase-node--done {
  color: var(--st-pass);
  border-color: transparent;
  background: var(--st-pass-soft);
}
.phase-node--active {
  color: var(--st-running);
  border-color: transparent;
  background: var(--st-running-soft);
}
.phase-node--failed {
  color: var(--st-fail);
  border-color: transparent;
  background: var(--st-fail-soft);
}
.phase-arrow {
  color: var(--text-3);
  font-size: 11px;
}
.hello-box {
  margin-top: 8px;
  font-size: 12px;
  color: var(--st-pass);
  background: var(--st-pass-soft);
  border-radius: var(--radius-sm);
  padding: 7px 10px;
}
.msg-feed {
  margin-top: 10px;
  max-height: 160px;
  overflow-y: auto;
  background: var(--bg-terminal);
  border-radius: var(--radius-sm);
  padding: 8px 10px;
  display: flex;
  flex-direction: column;
  gap: 3px;
}
.msg-row {
  display: flex;
  gap: 8px;
  font-size: 11px;
  color: #9fb0c3;
  white-space: nowrap;
  overflow: hidden;
}
.msg-ts {
  color: #556;
  flex-shrink: 0;
}
.msg-kind--send { color: #7dd3fc; }
.msg-kind--error { color: #f87171; }
.msg-kind--detached { color: #fbbf24; }
.msg-kind--log { color: #a5b4fc; }
.msg-text {
  overflow: hidden;
  text-overflow: ellipsis;
}
.proc-list {
  max-height: 420px;
  overflow-y: auto;
}
</style>
