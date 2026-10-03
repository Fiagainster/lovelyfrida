<script setup lang="ts">
import { computed, defineAsyncComponent, onMounted, onUnmounted } from "vue";
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
import { useDiagStore } from "@/stores/diagnostics";
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
// 非默认视图异步加载：TimelineView（连带 NDataTable 的虚拟滚动机制）等重组件
// 拆独立 chunk，首屏不为「可能永远不打开的视图」付包体
const TimelineView = defineAsyncComponent(() => import("@/views/TimelineView.vue"));
const DiffView = defineAsyncComponent(() => import("@/views/DiffView.vue"));
const TopologyView = defineAsyncComponent(() => import("@/views/TopologyView.vue"));
const TerminalView = defineAsyncComponent(() => import("@/views/TerminalView.vue"));
const LedgerConsole = defineAsyncComponent(() => import("@/components/LedgerConsole.vue"));
import PlaceholderView from "@/views/PlaceholderView.vue";

const app = useAppStore();
const pipeline = usePipelineStore();
const settings = useSettingsStore();
const sessionStore = useSessionStore();
const terminalStore = useTerminalStore();
const probeStore = useProbeStore();
const diag = useDiagStore();

const activeComponent = computed(() => {
  if (app.activeView === "pipeline") return PipelineView;
  if (app.activeView === "timeline") return TimelineView;
  if (app.activeView === "diff") return DiffView;
  if (app.activeView === "topology") return TopologyView;
  if (app.activeView === "terminal") return TerminalView;
  if (app.activeView === "ledger") return LedgerConsole;
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

// Tauri 事件监听的生命周期收口：此前 8 处 listen 全部丢弃 unlisten——生产无实害，
// 但 dev HMR 一次就多一份重复监听（pushTrace/pushMessage 双份处理）
const unlisteners: Array<() => void> = [];

onMounted(() => {
  diag.startActiveProbing();
  document.documentElement.setAttribute("data-theme", settings.theme);
  void settings.load();
  window.addEventListener("keydown", onKeydown);
  if (isTauri()) {
    // 关闭握手第一步：Rust 拦截 CloseRequested 后通知前端弹确认框；
    // frida 事件（message/detached/device_lost）+ 会话状态机事件 → store
    void import("@tauri-apps/api/event").then(async ({ listen }) => {
      unlisteners.push(
        await listen("close-requested", () => {
          app.closeDialogOpen = true;
        }),
      );
      unlisteners.push(await sessionStore.bindEvents(listen));
      unlisteners.push(await terminalStore.bindEvents(listen));
      unlisteners.push(await probeStore.bindEvents(listen));
    });
  }
});
onUnmounted(() => {
  window.removeEventListener("keydown", onKeydown);
  diag.stopActiveProbing();
  while (unlisteners.length) unlisteners.pop()?.();
});
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
          <div class="side-panel__header">
            诊断 / 详情
            <span v-if="diag.cards.length" style="font-weight: 400; font-size: 11px">
              （{{ diag.cards.length }} 张卡
              <a style="cursor: pointer; text-decoration: underline" @click="diag.restoreAll()">恢复忽略</a>）
            </span>
          </div>
          <div class="side-panel__body">
            <!-- 诊断卡片流（P2-1：规则引擎数据驱动，docs/05 48 条） -->
            <div v-if="diag.cards.length" style="display: flex; flex-direction: column; gap: 8px">
              <div v-for="d in diag.cards" :key="d.id" class="side-hint" style="flex-direction: column; align-items: stretch; gap: 4px">
                <div style="display: flex; align-items: center; gap: 6px">
                  <span :class="['status-chip', d.severity === 'block' ? 'status-chip--fail' : d.severity === 'warn' ? 'status-chip--warn' : 'status-chip--pending']">
                    {{ d.severity === "block" ? "阻断" : d.severity === "warn" ? "警告" : "提示" }}
                  </span>
                  <b>{{ d.ruleId }}</b>
                  <NButton size="tiny" quaternary style="margin-left: auto" @click="diag.ignore(d.ruleId)">忽略</NButton>
                </div>
                <span style="font-weight: 500">{{ d.title }}</span>
                <span>真因：{{ d.cause }}</span>
                <span v-for="(f, i) in d.fix" :key="i">{{ f }}</span>
                <span style="color: var(--text-3)">出处：{{ d.source }}</span>
              </div>
            </div>
            <template v-if="pipeline.doctor">
              <p style="margin: 12px 0 8px; font-size: 12px; color: var(--text-2)">
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
            <p v-if="!diag.cards.length && !pipeline.doctor" style="color: var(--text-3); font-size: 12px">
              诊断卡片将在此展示（症状 → 真因 → 处置 → 出处，文档05 · 48 条规则）。
              跑一次体检 / 附加会话 / 挂探针后，命中的规则会以卡片形式出现。
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
