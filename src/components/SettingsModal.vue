<script setup lang="ts">
import { ref, watch } from "vue";
import {
  NModal,
  NSwitch,
  NInput,
  NInputNumber,
  NButton,
  NTag,
  NSelect,
  useMessage,
} from "naive-ui";
import { api, type AppConfig } from "@/api";
import { useSettingsStore } from "@/stores/settings";

/**
 * 设置弹窗：体检可配置项（端口/自定义 adb/自动体检）+ 主题 + 只读根。
 * 保存走 update_config 整包提交（Rust 侧校验 + 审计）。
 */
const props = defineProps<{ show: boolean }>();
const emit = defineEmits<{ (e: "update:show", v: boolean): void }>();
const message = useMessage();
const settings = useSettingsStore();

const draft = ref<AppConfig | null>(null);
const saving = ref(false);
const newPort = ref<number | null>(null);
const newPath = ref("");
const newRoot = ref("");

const channelOptions = [
  { label: "auto（自动降级，当前先 B）", value: "auto" },
  { label: "a（Rust frida crate，M2+）", value: "a" },
  { label: "b（Python sidecar，主通道）", value: "b" },
  { label: "c（CLI 兜底）", value: "c" },
];

watch(
  () => props.show,
  (open) => {
    if (open && settings.config) {
      draft.value = JSON.parse(JSON.stringify(settings.config));
      newPort.value = null;
      newPath.value = "";
      newRoot.value = "";
    }
  },
);

function addPort() {
  const p = newPort.value;
  if (!p || p <= 0 || p > 65535 || draft.value?.doctor.emulator_ports.includes(p)) return;
  draft.value?.doctor.emulator_ports.push(p);
  newPort.value = null;
}
function addPath() {
  const t = newPath.value.trim();
  if (!t || draft.value?.doctor.adb_extra_paths.includes(t)) return;
  draft.value?.doctor.adb_extra_paths.push(t);
  newPath.value = "";
}
function addRoot() {
  const t = newRoot.value.trim();
  if (!t || draft.value?.read_only_roots.includes(t)) return;
  draft.value?.read_only_roots.push(t);
  newRoot.value = "";
}

async function save() {
  if (!draft.value) return;
  saving.value = true;
  try {
    const saved = await api.updateConfig(draft.value);
    settings.config = saved;
    if (saved.ui.theme !== settings.theme) {
      settings.theme = saved.ui.theme;
    }
    message.success("设置已保存");
    emit("update:show", false);
  } catch (e) {
    message.error(String(e));
  } finally {
    saving.value = false;
  }
}
</script>

<template>
  <NModal
    :show="props.show"
    preset="card"
    title="设置"
    style="width: 680px"
    :bordered="false"
    size="small"
    @update:show="emit('update:show', $event)"
  >
    <div v-if="draft" class="settings-body">
      <section class="settings-section">
        <h4>外观</h4>
        <div class="settings-row">
          <span>主题</span>
          <NSelect
            v-model:value="draft.ui.theme"
            :options="[
              { label: '深色', value: 'dark' },
              { label: '浅色', value: 'light' },
            ]"
            size="small"
            style="width: 140px"
          />
        </div>
      </section>

      <section class="settings-section">
        <h4>环境体检</h4>
        <div class="settings-row">
          <span>启动时自动运行快速体检</span>
          <NSwitch v-model:value="draft.doctor.auto_run_on_startup" size="small" />
        </div>
        <div class="settings-row" style="margin-top: 6px">
          <span>深度体检连接超时（秒，E-02 硬约束默认 15）</span>
          <NInputNumber
            v-model:value="draft.doctor.deep_connect_timeout_s"
            size="small"
            style="width: 110px"
            :min="1"
            :max="60"
            :show-button="false"
          />
        </div>
        <div class="settings-row" style="margin-top: 8px">模拟器 adb 端口（深度体检按此列表连接）</div>
        <div class="tag-editor">
          <NTag
            v-for="(p, i) in draft.doctor.emulator_ports"
            :key="`p${i}`"
            size="small"
            closable
            @close="draft.doctor.emulator_ports.splice(i, 1)"
          >
            {{ p }}
          </NTag>
          <span class="tag-add">
            <NInputNumber
              v-model:value="newPort"
              size="tiny"
              placeholder="端口"
              :show-button="false"
              style="width: 96px"
              @keydown.enter="addPort"
            />
            <NButton size="tiny" secondary @click="addPort">添加</NButton>
          </span>
        </div>
        <div class="settings-row" style="margin-top: 8px">自定义 adb 候选路径（优先于自动扫描）</div>
        <div class="tag-editor">
          <NTag
            v-for="(p, i) in draft.doctor.adb_extra_paths"
            :key="`a${i}`"
            size="small"
            closable
            @close="draft.doctor.adb_extra_paths.splice(i, 1)"
          >
            {{ p }}
          </NTag>
          <span class="tag-add">
            <NInput
              v-model:value="newPath"
              size="tiny"
              placeholder="D:\\...\\adb.exe"
              style="width: 240px"
              @keydown.enter="addPath"
            />
            <NButton size="tiny" secondary @click="addPath">添加</NButton>
          </span>
        </div>
      </section>

      <section class="settings-section">
        <h4>连接</h4>
        <div class="settings-row">
          <span>Frida 通道（D2：先做 B，A/C 兜底）</span>
          <NSelect
            v-model:value="draft.preferred_channel"
            :options="channelOptions"
            size="small"
            style="width: 230px"
          />
        </div>
        <div class="settings-row">
          <span>frida 端口</span>
          <NInputNumber
            v-model:value="draft.frida_port"
            size="small"
            style="width: 110px"
            :min="1024"
            :max="65535"
            :show-button="false"
          />
        </div>
      </section>

      <section class="settings-section">
        <h4>只读保护</h4>
        <div class="settings-row">检材只读根（对这些路径的写入将被直接拒绝，文档07）</div>
        <div class="tag-editor">
          <NTag
            v-for="(p, i) in draft.read_only_roots"
            :key="`r${i}`"
            size="small"
            type="warning"
            closable
            @close="draft.read_only_roots.splice(i, 1)"
          >
            {{ p }}
          </NTag>
          <span class="tag-add">
            <NInput
              v-model:value="newRoot"
              size="tiny"
              placeholder="D:\\evidence\\case-001"
              style="width: 240px"
              @keydown.enter="addRoot"
            />
            <NButton size="tiny" secondary @click="addRoot">添加</NButton>
          </span>
        </div>
      </section>
    </div>
    <div v-else class="settings-empty">
      设置加载中（或在浏览器预览模式下不可用——请在 Tauri 窗口内使用）。
    </div>

    <template #footer>
      <div class="settings-footer">
        <span class="settings-hint">只读根 = 机制不是提醒：命中即拒绝，无确认弹窗</span>
        <div>
          <NButton size="small" @click="emit('update:show', false)">取消</NButton>
          <NButton size="small" type="primary" :loading="saving" @click="save">保存</NButton>
        </div>
      </div>
    </template>
  </NModal>
</template>

<style scoped>
.settings-body {
  display: flex;
  flex-direction: column;
  gap: 16px;
}
.settings-section h4 {
  margin: 0 0 8px;
  font-size: 12px;
  color: var(--text-3);
  letter-spacing: 1px;
}
.settings-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  font-size: 13px;
  padding: 3px 0;
}
.tag-editor {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  align-items: center;
  padding: 6px 0;
}
.tag-add {
  display: inline-flex;
  gap: 6px;
  align-items: center;
}
.settings-footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.settings-footer > div {
  display: flex;
  gap: 8px;
}
.settings-hint {
  font-size: 11px;
  color: var(--text-3);
}
.settings-empty {
  padding: 32px 0;
  text-align: center;
  color: var(--text-3);
  font-size: 13px;
}
</style>
