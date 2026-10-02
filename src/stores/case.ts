import { defineStore } from "pinia";
import { ref } from "vue";

/**
 * 案件名单一真源（文档10 完善计划 P0-6）：
 * 档案（AppProfile）与台账（Findings）必须落同一个 case，否则 U9 复用链断开。
 * localStorage 持久化，重启后仍指向同一案件。
 */
export const useCaseStore = defineStore("case", () => {
  const STORAGE_KEY = "lovelyfrida.caseName";
  const DEFAULT_CASE = "默认案件";

  const caseName = ref(localStorage.getItem(STORAGE_KEY) ?? DEFAULT_CASE);

  function setCaseName(name: string) {
    caseName.value = name.trim() || DEFAULT_CASE;
    localStorage.setItem(STORAGE_KEY, caseName.value);
  }

  /** 台账/档案接口要求的兜底空串语义（与原「default」行为一致） */
  const apiCaseName = () => caseName.value.trim() || DEFAULT_CASE;

  return { caseName, setCaseName, apiCaseName };
});
