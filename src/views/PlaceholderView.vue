<script setup lang="ts">
import { useAppStore, VIEW_DEFS } from "@/stores/app";

const props = defineProps<{ view: string }>();
const def = VIEW_DEFS.find((v) => v.key === props.view);
const app = useAppStore();
</script>

<template>
  <div class="placeholder-view" style="height: 100%">
    <h2 style="margin: 0; color: var(--text-2)">{{ def?.name }}视图</h2>
    <span class="placeholder-view__milestone">{{ def?.milestone }} 里程碑交付</span>
    <span style="font-size: 12px; color: var(--text-3); max-width: 420px; text-align: center">
      {{ app.statusText }}
    </span>
  </div>
</template>
