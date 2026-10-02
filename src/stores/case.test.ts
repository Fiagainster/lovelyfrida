import { beforeEach, describe, expect, it } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { useCaseStore } from "@/stores/case";

/** case store 单测（A3）：案件名单一真源（P0-6） */

describe("case store · 案件名单一真源", () => {
  beforeEach(() => {
    localStorage.clear();
    setActivePinia(createPinia());
  });

  it("默认案件名，改名写回 localStorage", () => {
    const c = useCaseStore();
    expect(c.caseName).toBe("默认案件");
    c.setCaseName("测试案件A");
    expect(c.caseName).toBe("测试案件A");
    expect(localStorage.getItem("lovelyfrida.caseName")).toBe("测试案件A");
  });

  it("空名/空白回退默认；apiCaseName 永不返回空串", () => {
    const c = useCaseStore();
    c.setCaseName("   ");
    expect(c.caseName).toBe("默认案件");
    expect(c.apiCaseName()).toBe("默认案件");
  });

  it("localStorage 持久化恢复（重启后仍指向同一案件）", () => {
    localStorage.setItem("lovelyfrida.caseName", "上次案件");
    setActivePinia(createPinia());
    const c = useCaseStore();
    expect(c.caseName).toBe("上次案件");
  });
});
