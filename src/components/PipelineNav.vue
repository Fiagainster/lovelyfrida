<script setup lang="ts">
import { computed } from "vue";
import { ChevronBackOutline, ChevronForwardOutline } from "@vicons/ionicons5";
import { usePipelineStore } from "@/stores/pipeline";
import { useAppStore } from "@/stores/app";
import StatusLight from "@/components/StatusLight.vue";

/** 左栏 = 流水线导航（文档03）：8 节点，每节点 = 状态灯 + 名称 + 子项计数 */
const pipeline = usePipelineStore();
const app = useAppStore();

const collapsed = computed(() => !app.navExpanded);

function onSelect(key: string) {
  pipeline.selectedNode = key;
  app.switchView("pipeline");
}
</script>

<template>
  <nav class="pipeline-nav" :class="{ 'pipeline-nav--collapsed': collapsed }">
    <div class="pipeline-nav__header">{{ collapsed ? "" : "流水线 PIPELINE" }}</div>
    <div class="pipeline-nav__list">
      <div
        v-for="(n, i) in pipeline.nodes"
        :key="n.key"
        class="pipeline-node"
        :class="{ 'pipeline-node--active': pipeline.selectedNode === n.key }"
        :title="`${i + 1}. ${n.name} — ${n.summary}`"
        @click="onSelect(n.key)"
      >
        <span class="pipeline-node__index">{{ i + 1 }}</span>
        <StatusLight :status="n.status" :size="9" />
        <span v-if="!collapsed" class="pipeline-node__name">{{ n.name }}</span>
        <span v-if="!collapsed && n.subTotal > 0" class="pipeline-node__count">
          {{ n.subPass }}/{{ n.subTotal }}
        </span>
      </div>
    </div>
    <div class="pipeline-nav__footer">
      <div
        class="titlebar__win-btn"
        :title="collapsed ? '展开导航' : '收起导航'"
        @click="app.navExpanded = collapsed"
      >
        <ChevronForwardOutline v-if="collapsed" size="14" />
        <ChevronBackOutline v-else size="14" />
      </div>
    </div>
  </nav>
</template>
