<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useAppStore, VIEW_DEFS } from "@/stores/app";
import { usePipelineStore } from "@/stores/pipeline";
import { useTerminalStore } from "@/stores/terminal";
import { api } from "@/api";

/** 底部状态栏（文档03）：就绪状态 / 探针计数(M2) / 视图索引 / ⌘K 提示 */
const app = useAppStore();
const pipeline = usePipelineStore();
const terminal = useTerminalStore();

const viewName = computed(
  () => VIEW_DEFS.find((v) => v.key === app.activeView)?.name ?? "",
);
const deviceText = computed(() =>
  pipeline.deviceSerial ? `设备 ${pipeline.deviceSerial}` : "无设备",
);

// 版本号取自应用信息（tauri.conf / Cargo.toml），不再手写里程碑标签
const version = ref("");
onMounted(async () => {
  try {
    version.value = (await api.getAppInfo()).version;
  } catch {
    /* 浏览器预览模式下不可用，留空即可 */
  }
});
</script>

<template>
  <footer class="statusbar">
    <span class="status-light status-light--pass">
      <span class="status-light__dot" style="width: 8px; height: 8px" />
    </span>
    <span>{{ app.statusText }}</span>
    <span class="statusbar__hint">|</span>
    <span>{{ deviceText }}</span>
    <span class="statusbar__spacer" />
    <span class="statusbar__hint">
      <kbd>1</kbd>~<kbd>6</kbd> 切换视图 · 当前：{{ viewName }}
    </span>
    <span class="statusbar__hint">|</span>
    <span class="statusbar__hint">
      <kbd>Ctrl</kbd>+<kbd>K</kbd> 命令面板
    </span>
    <span class="statusbar__hint">|</span>
    <span class="statusbar__hint statusbar__btn" @click="terminal.toggle()">
      终端 {{ terminal.sessions.length > 0 ? `(${terminal.sessions.length})` : "" }}
    </span>
    <span class="statusbar__hint">|</span>
    <span v-if="version" class="statusbar__hint">v{{ version }}</span>
  </footer>
</template>
