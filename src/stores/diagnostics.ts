import { defineStore } from "pinia";
import { ref } from "vue";
import { api, type InjectionReport } from "@/api";
import { evaluateRules, type DiagCard, type DiagContext } from "@/diagnostics/rules";
import { usePipelineStore } from "@/stores/pipeline";
import { useSessionStore } from "@/stores/session";
import { useProbeStore } from "@/stores/probe";
import { isTauri } from "@/utils/env";

/**
 * 诊断卡片流（P2-1 数据驱动化）：规则表在 diagnostics/rules.ts（docs/05 全 48 条），
 * 本 store 只负责组装上下文、节流求值、忽略管理。卡片 = 症状谓词触发 → 真因 → 处置 → 出处。
 */
export const useDiagStore = defineStore("diag", () => {
  const cards = ref<DiagCard[]>([]);
  /** 用户忽略的规则 id（会话级） */
  const ignored = ref<string[]>([]);
  /** 最近一次内置爆破自测失败（C-07 硬门槛触发器） */
  const bruteSelfTestFailed = ref(false);
  /** 爆破引擎是否为生成 C 骨架（C-06 提示触发器） */
  const bruteEngineGenerateC = ref(false);
  /** 最近一次回灌报告（D 组谓词触发器） */
  const injection = ref<InjectionReport | null>(null);

  /** B1 主动探测发现（30s 巡检） */
  const activeFindings = ref<string[]>([]);

  const zeroHitSince = new Map<string, number>();
  const waitingSince = new Map<string, number>();

  let lastRun = 0;
  let pending: ReturnType<typeof setTimeout> | null = null;

  function buildContext(): DiagContext {
    const pipeline = usePipelineStore();
    const sessionStore = useSessionStore();
    const probe = useProbeStore();
    const now = Date.now();

    // 时间戳跟踪：waiting（P-05）/active 零命中（O-03）的持续时间判据
    for (const p of probe.probes) {
      if (p.status === "active" && p.hits === 0) {
        if (!zeroHitSince.has(p.id)) zeroHitSince.set(p.id, now);
      } else {
        zeroHitSince.delete(p.id);
      }
      if (p.status === "waiting") {
        if (!waitingSince.has(p.id)) waitingSince.set(p.id, now);
      } else {
        waitingSince.delete(p.id);
      }
    }
    // 已卸载探针的条目清掉（此前只在状态翻转时 delete，removeProbe 后 id 永久残留）
    const currentIds = new Set(probe.probes.map((p) => p.id));
    for (const id of [...zeroHitSince.keys()]) {
      if (!currentIds.has(id)) zeroHitSince.delete(id);
    }
    for (const id of [...waitingSince.keys()]) {
      if (!currentIds.has(id)) waitingSince.delete(id);
    }

    // 信号文本池：被动匹配（正则）的原料
    const signals: string[] = [];
    for (const c of pipeline.doctor?.checks ?? []) signals.push(...c.evidence);
    const s = sessionStore.session;
    if (s) signals.push(...s.evidence);
    for (const m of sessionStore.messages) signals.push(m.text);
    if (sessionStore.serverStatus) signals.push(...sessionStore.serverStatus.matrix);
    for (const p of probe.probes) if (p.lastError) signals.push(p.lastError);
    if (injection.value) {
      for (const st of injection.value.steps) signals.push(...st.evidence);
      signals.push(...injection.value.login_state_warnings);
    }

    return {
      doctor: pipeline.doctor,
      session: s,
      serverStatus: sessionStore.serverStatus,
      probes: probe.probes,
      injection: injection.value,
      signals: signals.map((x) => x.slice(0, 300)),
      activeFindings: activeFindings.value,
      zeroHitSince,
      waitingSince,
      bruteSelfTestFailed: bruteSelfTestFailed.value,
      bruteEngineGenerateC: bruteEngineGenerateC.value,
      now,
    };
  }

  function reevaluate() {
    lastRun = Date.now();
    cards.value = evaluateRules(buildContext()).filter(
      (c) => !ignored.value.includes(c.ruleId),
    );
  }

  /** 节流重算：trace 高频事件下最多 2s 一次（O-02 事件分层纪律） */
  function scheduleReevaluate() {
    if (pending !== null) return;
    const wait = Math.max(0, 2000 - (Date.now() - lastRun));
    pending = setTimeout(() => {
      pending = null;
      reevaluate();
    }, wait);
  }

  function ignore(ruleId: string) {
    if (!ignored.value.includes(ruleId)) ignored.value.push(ruleId);
    reevaluate();
  }

  function restoreAll() {
    ignored.value = [];
    reevaluate();
  }

  function reportBrute(selfTestFailed: boolean, engineGenerateC: boolean) {
    bruteSelfTestFailed.value = selfTestFailed;
    bruteEngineGenerateC.value = engineGenerateC;
    scheduleReevaluate();
  }

  function reportInjection(report: InjectionReport) {
    injection.value = report;
    scheduleReevaluate();
  }

  // ---------------- B1 主动探测（docs/05 第三条触发路径） ----------------
  // 被动匹配 + 状态谓词都依赖事件到来；「没有报错的那种错误」（会话静默死亡、
  // forward 悄悄失效）要靠定时主动查。30s 一轮，仅在 Tauri 运行时内执行。
  let probeTimer: ReturnType<typeof setInterval> | null = null;

  async function runActiveProbe() {
    if (!isTauri()) return;
    const sessionStore = useSessionStore();
    const probeStore = useProbeStore();
    if (sessionStore.session?.phase === "running") {
      try {
        const snap = await api.fridaSessionStatus();
        if (snap.phase !== "running") {
          if (!activeFindings.value.includes("session-drift")) {
            activeFindings.value = [...activeFindings.value, "session-drift"];
          }
        } else {
          activeFindings.value = activeFindings.value.filter((f) => f !== "session-drift");
        }
        // server/forward 矩阵刷新（S-05/S-02 状态谓词的最新输入）
        try {
          sessionStore.serverStatus = await api.fridaServerStatus();
        } catch {
          /* 刷新失败不影响本轮 */
        }
        // 探针状态巡检：零命中计时器不依赖探针控制台是否打开
        void probeStore.refreshStats();
      } catch {
        /* 状态查询失败：下一轮再试 */
      }
    } else {
      activeFindings.value = activeFindings.value.filter((f) => f !== "session-drift");
    }
    scheduleReevaluate();
  }

  function startActiveProbing() {
    if (probeTimer !== null) return;
    probeTimer = setInterval(() => void runActiveProbe(), 30_000);
    void runActiveProbe(); // 启动即查一轮
  }

  function stopActiveProbing() {
    if (probeTimer !== null) {
      clearInterval(probeTimer);
      probeTimer = null;
    }
  }

  return {
    cards,
    ignored,
    reevaluate,
    scheduleReevaluate,
    ignore,
    restoreAll,
    reportBrute,
    reportInjection,
    startActiveProbing,
    stopActiveProbing,
  };
});
