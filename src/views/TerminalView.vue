<script setup lang="ts">
import { ref } from "vue";
import { NButton, NInput, NTag } from "naive-ui";
import { useTerminalStore } from "@/stores/terminal";
import { useSessionStore } from "@/stores/session";

/**
 * 终端视图（文档03 视图6 · 阶段③落地）：
 * 终端本体是可升降抽屉（TerminalDrawer，任意视图 ⌘K/状态栏唤起）；
 * 本视图提供常驻入口：新建/管理 PTY 会话 + 手敲命令自动录 Step（M1 验收承诺）。
 */
const terminal = useTerminalStore();
const session = useSessionStore();
const serial = ref(terminal.lastSerial);

async function onCreate() {
  try {
    await terminal.create(serial.value.trim() || terminal.lastSerial);
  } catch (e) {
    /* store 内已 emit 到抽屉；错误经 message 由抽屉层展示 */
    console.error(String(e));
  }
}

function openDrawer() {
  terminal.toggle();
}
</script>

<template>
  <div class="workspace__view-inner" style="max-width: 720px">
    <div class="view-head">
      <div class="view-head__title">
        <div class="view-head__icon" style="background: var(--accent-soft)">⌨️</div>
        <div>
          <h2>原始终端</h2>
          <div class="view-head__sub">
            PTY 会话（adb -s &lt;serial&gt; shell）· 手敲命令自动录进 Recorder 并留审计 ·
            抽屉可从任意视图唤起
          </div>
        </div>
      </div>
    </div>

    <div class="card info-card">
      <h3>新建终端会话</h3>
      <div class="connect-row">
        <NInput v-model:value="serial" size="small" placeholder="serial（如 127.0.0.1:16384）" style="width: 240px" />
        <NButton size="small" type="primary" @click="onCreate">新建终端</NButton>
        <NButton size="small" secondary @click="openDrawer">
          打开终端抽屉（{{ terminal.sessions.length }} 个会话）
        </NButton>
      </div>
      <p class="muted" style="margin: 8px 0 0; font-size: 11px">
        默认 serial 跟随体检/会话（当前：{{ session.session?.device ?? "未连接" }}）；MuMu 12 默认 16384。
      </p>
    </div>

    <div class="card info-card">
      <h3>活动会话</h3>
      <table v-if="terminal.sessions.length" class="plain-table">
        <thead>
          <tr><th style="width: 70px">id</th><th>serial</th><th style="width: 120px" /></tr>
        </thead>
        <tbody>
          <tr v-for="s in terminal.sessions" :key="s.id">
            <td class="mono">term#{{ s.id }}</td>
            <td class="mono" style="font-size: 12px">{{ s.serial }}</td>
            <td>
              <NTag size="small" :bordered="false">使用抽屉操作</NTag>
            </td>
          </tr>
        </tbody>
      </table>
      <p v-else class="muted" style="margin: 6px 0 0">
        暂无终端会话。终端抽屉里可多开（每个 tab 一个 PTY），关闭窗口即结束会话。
      </p>
    </div>
  </div>
</template>
