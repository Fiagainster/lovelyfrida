<script setup lang="ts">
import { nextTick, onMounted, ref, watch } from "vue";
import { NButton, NInput, useMessage } from "naive-ui";
import { AddOutline, CloseOutline, ChevronDownOutline } from "@vicons/ionicons5";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { useTerminalStore } from "@/stores/terminal";
import { api } from "@/api";
import { useSessionStore } from "@/stores/session";

/** 底部终端抽屉：xterm.js ↔ portable-pty（adb shell），文档03「三层下钻」的 L3 入口 */
const store = useTerminalStore();
const sessionStore = useSessionStore();
const message = useMessage();

const serialInput = ref(store.lastSerial);
const creating = ref(false);
const bodyEl = ref<HTMLElement | null>(null);

interface TermEntry {
  term: Terminal;
  fit: FitAddon;
}
const terms = new Map<number, TermEntry>();

function b64ToBytes(b64: string): Uint8Array {
  const bin = atob(b64);
  const u8 = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) u8[i] = bin.charCodeAt(i);
  return u8;
}

function mountTerm(id: number) {
  if (terms.has(id)) return;
  const el = bodyEl.value?.querySelector(`#term-${id}`) as HTMLElement | null;
  if (!el) return;
  const term = new Terminal({
    fontSize: 12,
    fontFamily: "Cascadia Mono, Consolas, monospace",
    cursorBlink: true,
    theme: { background: "#0a0d10", foreground: "#c9d4e0" },
  });
  const fit = new FitAddon();
  term.loadAddon(fit);
  term.open(el);
  try {
    fit.fit();
  } catch {
    /* 容器未布局完成时忽略 */
  }
  term.onData((d) => void apiWrite(id, d));
  term.onResize(({ cols, rows }) => void api.terminalResize(id, cols, rows).catch(() => {}));
  store.onData(id, (b64) => term.write(b64ToBytes(b64)));
  store.onClosed(id, () => {
    terms.get(id)?.term.dispose();
    terms.delete(id);
  });
  terms.set(id, { term, fit });
  void api.terminalResize(id, term.cols, term.rows).catch(() => {});
  term.focus();
}

async function apiWrite(id: number, d: string) {
  try {
    await api.terminalWrite(id, d);
  } catch {
    /* 会话已关闭 */
  }
}

async function onCreate() {
  creating.value = true;
  try {
    await store.create(serialInput.value.trim() || "127.0.0.1:16384");
    await nextTick();
    if (store.activeId != null) mountTerm(store.activeId);
  } catch (e) {
    message.error(String(e));
  } finally {
    creating.value = false;
  }
}

watch(
  () => store.activeId,
  async (id) => {
    await nextTick();
    if (id != null) {
      mountTerm(id);
      terms.get(id)?.fit.fit();
      terms.get(id)?.term.focus();
    }
  },
);

watch(
  () => store.drawerOpen,
  async (open) => {
    if (open) {
      await nextTick();
      const id = store.activeId;
      if (id != null) {
        mountTerm(id);
        terms.get(id)?.fit.fit();
      }
    }
  },
);

onMounted(() => {
  if (sessionStore.serverStatus?.device_serial) {
    serialInput.value = sessionStore.serverStatus.device_serial;
  }
});
</script>

<template>
  <Teleport to="body">
    <div v-if="store.drawerOpen" class="term-drawer card">
      <div class="term-drawer__head">
        <div class="term-drawer__tabs">
          <div
            v-for="s in store.sessions"
            :key="s.id"
            class="term-tab"
            :class="{ 'term-tab--active': store.activeId === s.id }"
            @click="store.activeId = s.id"
          >
            <span>term#{{ s.id }} · {{ s.serial }}</span>
            <span
              class="term-tab__close"
              @click.stop="store.close(s.id)"
            >
              <CloseOutline size="10" />
            </span>
          </div>
          <NInput
            v-model:value="serialInput"
            size="tiny"
            placeholder="serial"
            style="width: 150px"
          />
          <NButton size="tiny" secondary :loading="creating" @click="onCreate">
            <template #icon><AddOutline /></template>
            新建终端
          </NButton>
        </div>
        <div class="term-drawer__collapse" title="收起（会话保持）" @click="store.toggle()">
          <ChevronDownOutline size="14" />
        </div>
      </div>
      <div ref="bodyEl" class="term-drawer__body">
        <div
          v-for="s in store.sessions"
          :key="s.id"
          v-show="store.activeId === s.id"
          :id="`term-${s.id}`"
          class="term-mount"
        />
        <div v-if="store.sessions.length === 0" class="term-empty">
          点「新建终端」建立 adb shell PTY 会话（serial 可改）
        </div>
      </div>
    </div>
  </Teleport>
</template>

<style>
.term-drawer {
  position: fixed;
  left: 0;
  right: 0;
  bottom: var(--statusbar-h);
  height: 40vh;
  min-height: 220px;
  z-index: 90;
  border-radius: 0;
  border-left: none;
  border-right: none;
  border-bottom: none;
  display: flex;
  flex-direction: column;
  box-shadow: 0 -6px 24px rgba(0, 0, 0, 0.35);
}
.term-drawer__head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 4px 8px;
  border-bottom: 1px solid var(--border-1);
  background: var(--bg-titlebar);
}
.term-drawer__tabs {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
}
.term-tab {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: 11px;
  font-family: var(--font-mono);
  padding: 3px 9px;
  border-radius: var(--radius-sm);
  cursor: pointer;
  color: var(--text-2);
  background: var(--bg-elevated);
  border: 1px solid var(--border-1);
}
.term-tab--active {
  color: var(--text-1);
  background: var(--accent-soft);
  border-color: transparent;
}
.term-tab__close {
  color: var(--text-3);
  display: flex;
}
.term-tab__close:hover {
  color: var(--st-fail);
}
.term-drawer__collapse {
  cursor: pointer;
  color: var(--text-2);
  padding: 4px 8px;
  display: flex;
}
.term-drawer__collapse:hover {
  color: var(--text-1);
}
.term-drawer__body {
  flex: 1;
  min-height: 0;
  position: relative;
  background: var(--bg-terminal);
}
.term-mount {
  position: absolute;
  inset: 0;
  padding: 4px 8px;
}
.term-empty {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-3);
  font-size: 12px;
}
</style>
