import { defineStore } from "pinia";
import { computed, ref } from "vue";
import { api, type CheckResult, type CheckStatus, type DoctorReport } from "@/api";

/** 流水线 8 节点（文档03：左栏=流水线导航，不是菜单） */
export interface PipelineNodeState {
  key: string;
  name: string;
  status: CheckStatus;
  /** 子项计数：通过/总数 */
  subTotal: number;
  subPass: number;
  summary: string;
}

function node(key: string, name: string): PipelineNodeState {
  return { key, name, status: "pending", subTotal: 0, subPass: 0, summary: "未开始" };
}

export const usePipelineStore = defineStore("pipeline", () => {
  const nodes = ref<PipelineNodeState[]>([
    node("doctor", "环境体检"),
    node("connect", "设备连接"),
    node("load", "应用装载"),
    node("inject", "数据回灌"),
    node("probe", "探针注入"),
    node("observe", "观测"),
    node("restore", "还原"),
    node("archive", "归档"),
  ]);

  const doctor = ref<DoctorReport | null>(null);
  const doctorRunning = ref(false);
  const doctorError = ref<string | null>(null);
  const selectedNode = ref("doctor");

  const deviceSerial = computed(() => doctor.value?.device_serial ?? null);

  function applyDoctorReport(report: DoctorReport) {
    doctor.value = report;
    // 诊断规则引擎重算（E 组谓词依赖体检结果；低频动作直接重算）
    void import("@/stores/diagnostics").then(({ useDiagStore }) => useDiagStore().reevaluate());
    const checks: CheckResult[] = report.checks;
    const pass = checks.filter((c) => c.status === "pass").length;
    const warn = checks.filter((c) => c.status === "warn").length;
    const fail = checks.filter((c) => c.status === "fail").length;
    const n1 = nodes.value[0];
    n1.subTotal = checks.length;
    n1.subPass = pass;
    n1.summary = `通过 ${pass}/${checks.length}`;
    n1.status = fail > 0 ? "fail" : warn > 0 ? "warn" : "pass";

    // 节点2 设备连接：以体检第 3 项（adb 能连）为准
    const conn = checks.find((c) => c.id === "CHK-03");
    const n2 = nodes.value[1];
    if (conn) {
      n2.status = conn.status;
      n2.summary = report.device_serial ? `已连接 ${report.device_serial}` : "未连接";
      n2.subTotal = 1;
      n2.subPass = conn.status === "pass" ? 1 : 0;
    } else {
      n2.status = "skip";
      n2.summary = "体检未覆盖";
    }
  }

  async function runDoctor(deep = false) {
    doctorRunning.value = true;
    doctorError.value = null;
    const n1 = nodes.value[0];
    n1.status = "running";
    n1.summary = deep ? "深度体检中…" : "快速体检中…";
    try {
      const report = await api.doctorRun(deep);
      applyDoctorReport(report);
    } catch (e) {
      doctorError.value = String(e);
      n1.status = "fail";
      n1.summary = "体检执行失败";
    } finally {
      doctorRunning.value = false;
    }
  }

  return {
    nodes,
    doctor,
    doctorRunning,
    doctorError,
    selectedNode,
    deviceSerial,
    runDoctor,
    applyDoctorReport,
  };
});
