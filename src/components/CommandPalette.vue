<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useAppStore, VIEW_DEFS } from "@/stores/app";
import { usePipelineStore } from "@/stores/pipeline";
import { useSettingsStore } from "@/stores/settings";

interface PaletteCmd {
  id: string;
  label: string;
  hint: string;
  run: () => void;
}

/** ⌘K 命令面板（文档03：键盘优先，全程无鼠标） */
const app = useAppStore();
const pipeline = usePipelineStore();
const settings = useSettingsStore();

const query = ref("");
const inputEl = ref<HTMLInputElement | null>(null);

const commands = computed<PaletteCmd[]>(() => [
  ...VIEW_DEFS.map((v, i) => ({
    id: `view-${v.key}`,
    label: `切换到「${v.name}」视图`,
    hint: `数字键 ${i + 1}`,
    run: () => app.switchView(v.key),
  })),
  {
    id: "doctor-run",
    label: "快速体检（本地项 + adb 现状，<2s）",
    hint: "Doctor",
    run: () => {
      pipeline.selectedNode = "doctor";
      app.switchView("pipeline");
      void pipeline.runDoctor(false);
    },
  },
  {
    id: "doctor-deep",
    label: "深度体检（连接重试 + offline 自愈，兜底）",
    hint: "Doctor Deep",
    run: () => {
      pipeline.selectedNode = "doctor";
      app.switchView("pipeline");
      void pipeline.runDoctor(true);
    },
  },
  {
    id: "settings",
    label: "打开设置（体检端口 / adb / 只读根）",
    hint: "Settings",
    run: () => {
      app.settingsOpen = true;
    },
  },
  {
    id: "theme-toggle",
    label: `切换主题（当前：${settings.theme === "dark" ? "深色" : "浅色"}）`,
    hint: "文档03 深色/浅色",
    run: () => settings.setTheme(settings.theme === "dark" ? "light" : "dark"),
  },
]);

const filtered = computed(() => {
  const q = query.value.trim().toLowerCase();
  if (!q) return commands.value;
  return commands.value.filter(
    (c) => c.label.toLowerCase().includes(q) || c.hint.toLowerCase().includes(q),
  );
});

function exec(cmd: PaletteCmd) {
  cmd.run();
  app.paletteOpen = false;
}

watch(
  () => app.paletteOpen,
  (open) => {
    if (open) {
      query.value = "";
      setTimeout(() => inputEl.value?.focus(), 50);
    }
  },
);
</script>

<template>
  <Teleport to="body">
    <div v-if="app.paletteOpen" class="palette-mask" @click.self="app.paletteOpen = false">
      <div class="palette card">
        <input
          ref="inputEl"
          v-model="query"
          class="palette__input"
          placeholder="输入命令或视图名…（Esc 关闭）"
          @keydown.esc="app.paletteOpen = false"
          @keydown.enter="filtered.length > 0 && exec(filtered[0])"
        />
        <div class="palette__list">
          <div
            v-for="cmd in filtered"
            :key="cmd.id"
            class="palette__item"
            @click="exec(cmd)"
          >
            <span>{{ cmd.label }}</span>
            <span class="palette__hint">{{ cmd.hint }}</span>
          </div>
          <div v-if="filtered.length === 0" class="palette__empty">无匹配命令</div>
        </div>
      </div>
    </div>
  </Teleport>
</template>

<style>
.palette-mask {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.45);
  z-index: 200;
  display: flex;
  justify-content: center;
  align-items: flex-start;
  padding-top: 12vh;
  backdrop-filter: blur(2px);
}
.palette {
  width: 540px;
  max-width: 90vw;
  box-shadow: var(--shadow-2);
  overflow: hidden;
}
.palette__input {
  width: 100%;
  border: none;
  outline: none;
  background: var(--bg-input);
  color: var(--text-1);
  padding: 13px 16px;
  font-size: 14px;
  border-bottom: 1px solid var(--border-1);
}
.palette__input::placeholder {
  color: var(--text-3);
}
.palette__list {
  max-height: 320px;
  overflow-y: auto;
  padding: 6px;
}
.palette__item {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 9px 12px;
  border-radius: var(--radius-sm);
  cursor: pointer;
  color: var(--text-1);
  font-size: 13px;
}
.palette__item:hover {
  background: var(--accent-soft);
}
.palette__hint {
  font-size: 11px;
  color: var(--text-3);
}
.palette__empty {
  padding: 16px;
  text-align: center;
  color: var(--text-3);
}
</style>
