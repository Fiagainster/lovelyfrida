import { defineStore } from "pinia";
import { ref } from "vue";

/** 六视图（文档03：主工作区六视图切换） */
export type ViewKey = "pipeline" | "timeline" | "diff" | "topology" | "terminal" | "ledger";

export const VIEW_DEFS: { key: ViewKey; name: string; milestone: string }[] = [
  { key: "pipeline", name: "流水线", milestone: "M0" },
  { key: "timeline", name: "时间轴", milestone: "M2" },
  { key: "diff", name: "差分", milestone: "M3" },
  { key: "topology", name: "拓扑", milestone: "M3" },
  { key: "terminal", name: "终端", milestone: "M1" },
  { key: "ledger", name: "档案", milestone: "M5" },
];

export const useAppStore = defineStore("app", () => {
  const activeView = ref<ViewKey>("pipeline");
  const paletteOpen = ref(false);
  const closeDialogOpen = ref(false);
  const settingsOpen = ref(false);
  const statusText = ref("就绪");
  const navExpanded = ref(true);

  function switchView(v: ViewKey) {
    activeView.value = v;
  }

  return {
    activeView,
    paletteOpen,
    closeDialogOpen,
    settingsOpen,
    statusText,
    navExpanded,
    switchView,
  };
});
