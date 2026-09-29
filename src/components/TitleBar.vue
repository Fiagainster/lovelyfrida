<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { RemoveOutline, SquareOutline, CopyOutline, CloseOutline, SettingsOutline } from "@vicons/ionicons5";
import { isTauri } from "@/utils/env";
import { usePipelineStore } from "@/stores/pipeline";
import { useSettingsStore } from "@/stores/settings";
import { useAppStore } from "@/stores/app";

/**
 * 自定义标题栏（LovelyMem 模式）：drag-region + 自绘三键；
 * 关闭走握手协议：Rust 拦截 CloseRequested → 前端确认 → 优雅关停。
 */
const pipeline = usePipelineStore();
const settings = useSettingsStore();
const app = useAppStore();

const maximized = ref(false);
let unlisten: (() => void) | null = null;

const overallBadge = computed(() => {
  const d = pipeline.doctor;
  if (!d) return { text: "未体检", cls: "" };
  const map: Record<string, { text: string; cls: string }> = {
    pass: { text: "体检通过", cls: "titlebar__badge--ok" },
    warn: { text: "体检警告", cls: "titlebar__badge--warn" },
    fail: { text: "体检失败", cls: "titlebar__badge--fail" },
    skip: { text: "未体检", cls: "" },
  };
  return map[d.overall] ?? { text: "未体检", cls: "" };
});

async function winMaxToggle() {
  if (!isTauri()) return;
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  await getCurrentWindow().toggleMaximize();
  maximized.value = await getCurrentWindow().isMaximized();
}

async function winMinimize() {
  if (!isTauri()) return;
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  await getCurrentWindow().minimize();
}

async function winClose() {
  if (!isTauri()) return;
  // 触发 Rust CloseRequested 拦截 → 弹出确认握手
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  await getCurrentWindow().close();
}

onMounted(async () => {
  if (!isTauri()) return;
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  const win = getCurrentWindow();
  maximized.value = await win.isMaximized();
  // 窗口状态跟踪（最大/还原图标切换）
  const u1 = await win.onResized(async () => {
    maximized.value = await win.isMaximized();
  });
  unlisten = u1;
});
onUnmounted(() => unlisten?.());
</script>

<template>
  <header class="titlebar">
    <div class="titlebar__drag" data-tauri-drag-region>
      <div class="titlebar__logo">LF</div>
      <span class="titlebar__title" data-tauri-drag-region>LovelyFrida</span>
      <div class="titlebar__badges">
        <span class="titlebar__badge" :class="overallBadge.cls">{{ overallBadge.text }}</span>
        <span class="titlebar__badge" :title="'当前主题（文档03：深/浅两主题，状态色语义一致）'">
          {{ settings.theme === "dark" ? "深色" : "浅色" }}
        </span>
      </div>
    </div>
    <div class="titlebar__actions">
      <div class="titlebar__win-btn" title="设置（体检端口 / 自定义 adb / 只读根）" @click="app.settingsOpen = true">
        <SettingsOutline size="13" />
      </div>
      <div class="titlebar__win-btn" title="最小化" @click="winMinimize">
        <RemoveOutline size="13" />
      </div>
      <div class="titlebar__win-btn" :title="maximized ? '还原' : '最大化'" @click="winMaxToggle">
        <CopyOutline v-if="maximized" size="11" />
        <SquareOutline v-else size="11" />
      </div>
      <div class="titlebar__win-btn titlebar__win-btn--close" title="关闭" @click="winClose">
        <CloseOutline size="13" />
      </div>
    </div>
  </header>
</template>
