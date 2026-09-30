<script setup lang="ts">
import { computed, onMounted, onUnmounted } from "vue";
import {
  NConfigProvider,
  NModal,
  NMessageProvider,
  darkTheme,
  zhCN,
  dateZhCN,
} from "naive-ui";
import { useAppStore, VIEW_DEFS, type ViewKey } from "@/stores/app";
import { usePipelineStore } from "@/stores/pipeline";
import { useSessionStore } from "@/stores/session";
import { useTerminalStore } from "@/stores/terminal";
import { useProbeStore } from "@/stores/probe";
import { useSettingsStore } from "@/stores/settings";
import { api } from "@/api";
import { isTauri } from "@/utils/env";
import TitleBar from "@/components/TitleBar.vue";
import PipelineNav from "@/components/PipelineNav.vue";
import StatusBar from "@/components/StatusBar.vue";
import CommandPalette from "@/components/CommandPalette.vue";
import SettingsModal from "@/components/SettingsModal.vue";
import TerminalDrawer from "@/components/TerminalDrawer.vue";
import PipelineView from "@/views/PipelineView.vue";
import TimelineView from "@/views/TimelineView.vue";
import PlaceholderView from "@/views/PlaceholderView.vue";

const app = useAppStore();
const pipeline = usePipelineStore();
const settings = useSettingsStore();
const sessionStore = useSessionStore();
const terminalStore = useTerminalStore();
const probeStore = useProbeStore();

const activeComponent = computed(() => {
  if (app.activeView === "pipeline") return PipelineView;
  if (app.activeView === "timeline") return TimelineView;
  return PlaceholderView;
});

async function doConfirmClose() {
  app.closeDialogOpen = false;
  try {
    await api.confirmClose();
  } catch {
    /* Rust 端退出流程接管 */
  }
}

// ---------- 键盘优先（文档03：⌘K 面板、数字 1~6 切视图） ----------
function onKeydown(e: KeyboardEvent) {
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k") {
    e.preventDefault();
    app.paletteOpen = !app.paletteOpen;
    return;
  }
  if (app.paletteOpen) return;
  const target = e.target as HTMLElement | null;
  if (target && ["INPUT", "TEXTAREA"].includes(target.tagName)) return;
  const num = Number(e.key);
  if (num >= 1 && num <= 6) {
    e.preventDefault();
    app.switchView(VIEW_DEFS[num - 1].key as ViewKey);
  }
}

onMounted(() => {
  document.documentElement.setAttribute("data-theme", settings.theme);
  void settings.load();
  window.addEventListener("keydown", onKeydown);
  if (isTauri()) {
    // 关闭握手第一步：Rust 拦截 CloseRequested 后通知前端弹确认框
    import("@tauri-apps/api/event").then(({ listen }) => {
      listen("close-requested", () => {
        app.closeDialogOpen = true;
      });
    });
    // frida 事件（message/detached/device_lost）+ 会话状态机事件 → store
    import("@tauri-apps/api/event").then(({ listen }) => {
      void sessionStore.bindEvents(listen);
      void terminalStore.bindEvents(listen);
      void probeStore.bindEvents(listen);
    });
  }
});
onUnmounted(() => window.removeEventListener("keydown", onKeydown));
</script>

<template>
  <NConfigProvider
    :theme="settings.theme === 'dark' ? darkTheme : null"
    :locale="zhCN"
    :date-locale="dateZhCN"
    style="height: 100%"
  >
    <NMessageProvider>
    <div class="app-shell">
      <TitleBar />
      <div class="main-row">
        <PipelineNav />
        <main class="workspace">
          <div class="workspace__view">
            <component :is="activeComponent" :view="app.activeView" />
          </div>
        </main>
        <aside class="side-panel">
          <div class="side-panel__header">诊断 / 详情</div>
          <div class="side-panel__body">
            <template v-if="pipeline.doctor">
              <p style="margin: 0 0 8px; font-size: 12px; color: var(--text-2)">
                体检结论：{{ pipeline.doctor.overall }}
              </p>
              <div
                v-for="c in pipeline.doctor.checks.filter((c) => c.fix)"
                :key="c.id"
                class="side-hint"
              >
                <b>{{ c.name }}</b>
                <span>{{ c.fix }}</span>
              </div>
            </template>
            <p v-else style="color: var(--text-3); font-size: 12px">
              诊断卡片将在此展示（症状 → 真因 → 处置 → 出处，文档05）。
              诊断引擎 v1 于 M2 交付。
            </p>
          </div>
        </aside>
      </div>
      <StatusBar />

      <!-- 关闭确认握手（LovelyMem 协议：Rust 拦截 + 10s 超时保底） -->
      <NModal
        v-model:show="app.closeDialogOpen"
        preset="dialog"
        type="warning"
        title="退出 LovelyFrida？"
        content="将断开设备会话并清理子进程后退出。确认关闭？"
        positive-text="确认退出"
        negative-text="取消"
        @positive-click="doConfirmClose"
      />

      <CommandPalette />
      <TerminalDrawer />
      <SettingsModal v-model:show="app.settingsOpen" />
    </div>
    </NMessageProvider>
  </NConfigProvider>
</template>

<style>
.side-hint {
  border: 1px solid var(--border-1);
  border-left: 3px solid var(--st-warn);
  border-radius: var(--radius-sm);
  padding: 8px 10px;
  margin-bottom: 8px;
  font-size: 12px;
  display: flex;
  flex-direction: column;
  gap: 4px;
  background: var(--bg-elevated);
}
.side-hint b {
  font-size: 12px;
  color: var(--text-1);
}
.side-hint span {
  color: var(--text-2);
}
</style>
