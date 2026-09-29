<script setup lang="ts">
import { computed } from "vue";
import type { CheckStatus } from "@/api";

/**
 * 六态状态灯（文档03 状态语义，全场统一）：
 * ○灰未开始 / ◐蓝脉冲进行中 / ●绿通过 / ◑黄警告 / ✖红失败 / ⊘空跳过
 * 颜色不独担信息：形状 + 文字 + 计数配合。
 */
const props = withDefaults(
  defineProps<{
    status: CheckStatus;
    text?: string;
    size?: number;
    showText?: boolean;
  }>(),
  { size: 10, showText: false },
);

const TEXTS: Record<CheckStatus, string> = {
  pending: "未开始",
  running: "进行中",
  pass: "通过",
  warn: "警告",
  fail: "失败",
  skip: "跳过",
};

const cls = computed(() => `status-light--${props.status}`);
const label = computed(() => props.text ?? TEXTS[props.status]);
</script>

<template>
  <span class="status-light" :class="cls" :title="label">
    <span
      class="status-light__dot"
      :style="{ width: size + 'px', height: size + 'px' }"
    />
    <span v-if="showText" class="status-light__text">{{ label }}</span>
  </span>
</template>
