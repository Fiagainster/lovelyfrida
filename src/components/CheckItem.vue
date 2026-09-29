<script setup lang="ts">
import { ref } from "vue";
import { NButton } from "naive-ui";
import { ChevronForwardOutline, CopyOutline } from "@vicons/ionicons5";
import type { CheckResult } from "@/api";
import StatusLight from "@/components/StatusLight.vue";

/**
 * 单条检查项（文档03）：状态灯 + 名称 + 规则引用；
 * 展开后 = 证据原文 + 修复建议 + 等价命令（L2 下钻，可复制）。
 */
const props = defineProps<{ check: CheckResult }>();
const open = ref(false);
const copied = ref(false);

async function copyCommand() {
  if (!props.check.command) return;
  try {
    await navigator.clipboard.writeText(props.check.command);
    copied.value = true;
    setTimeout(() => (copied.value = false), 1200);
  } catch {
    /* 剪贴板不可用时忽略 */
  }
}
</script>

<template>
  <div class="check-item">
    <div class="check-item__head" @click="open = !open">
      <StatusLight :status="check.status" :size="10" />
      <span class="check-item__name">{{ check.name }}</span>
      <span v-if="check.rule" class="check-item__rule">{{ check.rule }}</span>
      <span
        class="check-item__chevron"
        :class="{ 'check-item__chevron--open': open }"
      >
        <ChevronForwardOutline size="12" />
      </span>
    </div>
    <div v-if="open" class="check-item__body">
      <pre class="check-item__evidence">{{ check.evidence.join("\n") || "（无证据输出）" }}</pre>
      <div v-if="check.fix" class="check-item__fix">处置建议：{{ check.fix }}</div>
      <div v-if="check.command" class="check-item__cmd">
        <code>{{ check.command }}</code>
        <NButton size="tiny" quaternary @click="copyCommand">
          <template #icon><CopyOutline /></template>
          {{ copied ? "已复制" : "复制" }}
        </NButton>
      </div>
    </div>
  </div>
</template>
