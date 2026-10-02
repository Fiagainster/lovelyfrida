<script setup lang="ts">
import { computed } from "vue";
import { NButton, NTag } from "naive-ui";
import { useAppStore } from "@/stores/app";
import { usePipelineStore } from "@/stores/pipeline";
import { useSessionStore } from "@/stores/session";
import { useProbeStore } from "@/stores/probe";
import { useTerminalStore } from "@/stores/terminal";
import StatusLight from "@/components/StatusLight.vue";

/**
 * 拓扑视图（文档03「钥匙链」· 阶段③落地）：
 * 设备 → frida-server → forward → 会话(进程) → agent/探针 的链路一览。
 * 每一环的状态灯都来自探测结果（状态灯由证据点亮，文档03 纪律），点击下钻到对应视图。
 */
const app = useAppStore();
const pipeline = usePipelineStore();
const session = useSessionStore();
const probe = useProbeStore();
const terminal = useTerminalStore();

const connCheck = computed(() => pipeline.doctor?.checks.find((c) => c.id === "CHK-03"));
const versionCheck = computed(() => pipeline.doctor?.checks.find((c) => c.id === "CHK-08"));

interface ChainNode {
  key: string;
  title: string;
  status: "pending" | "pass" | "warn" | "fail" | "running" | "skip";
  lines: string[];
  view: "pipeline" | "timeline" | "diff" | "topology" | "terminal" | "ledger" | null;
  viewLabel?: string;
}

const chain = computed<ChainNode[]>(() => {
  const ss = session.serverStatus;
  const s = session.session;
  const serial = pipeline.deviceSerial ?? ss?.device_serial ?? null;
  const nodes: ChainNode[] = [
    {
      key: "device",
      title: "① 设备（adb）",
      status: serial ? "pass" : pipeline.doctor ? (connCheck.value?.status ?? "pending") : "pending",
      lines: [
        serial ? `serial = ${serial}` : "未连接（体检可自动扫端口）",
        pipeline.doctor?.adb_path ? `adb = ${pipeline.doctor.adb_path}` : "adb 未探测",
      ],
      view: "pipeline",
      viewLabel: "去体检/连接",
    },
    {
      key: "server",
      title: "② frida-server（设备端）",
      status:
        ss?.server_running === true ? "pass" : ss?.server_running === false ? "fail" : "pending",
      lines: [
        ss?.device_server_version
          ? `运行版本实测 = ${ss.device_server_version}`
          : "版本未实测（会话链路自动推送）",
        ss?.client_version ? `客户端 = ${ss.client_version}` : "客户端版本未知",
        ss?.server_running === true
          ? `实测监听 :${ss.port}`
          : "未监听（假绿灯防护：以 ss -tlnp 为准）",
      ],
      view: "pipeline",
      viewLabel: "去会话控制台",
    },
    {
      key: "forward",
      title: "③ adb forward",
      status:
        ss?.forward_established === true
          ? "pass"
          : ss?.forward_established === false
            ? "fail"
            : "pending",
      lines: [
        s?.forward_host_port
          ? `主机端口 = 127.0.0.1:${s.forward_host_port}（S-06 自动换 port）`
          : "未建立",
      ],
      view: "pipeline",
      viewLabel: "去会话控制台",
    },
    {
      key: "session",
      title: "④ 会话（附加目标）",
      status:
        s?.phase === "running"
          ? "pass"
          : s?.phase === "failed"
            ? "fail"
            : s?.phase === "idle" || s?.phase === "stopped"
              ? "pending"
              : "running",
      lines: [
        s?.target ? `target = ${s.target}` : "未附加",
        s?.channel ? `通道 = ${s.channel}${s.channel === "C" ? "（CLI 兜底，仅观测）" : "（Python sidecar）"}` : "",
        s?.phase ? `phase = ${s.phase}` : "",
      ].filter(Boolean),
      view: "pipeline",
      viewLabel: "去会话控制台",
    },
    {
      key: "agent",
      title: "⑤ agent（探针/脚本）",
      status:
        s?.phase !== "running"
          ? "pending"
          : probe.probes.some((p) => p.status === "error")
            ? "fail"
            : probe.probes.length > 0
              ? "pass"
              : "warn",
      lines: [
        `探针 ${probe.probes.length} 个：active ${probe.probes.filter((p) => p.status === "active").length} / waiting ${probe.probes.filter((p) => p.status === "waiting").length} / error ${probe.probes.filter((p) => p.status === "error").length}`,
        `事件 ${probe.trace.length} 条（内存环）`,
        s?.script_id != null ? `script#${s.script_id}` : "脚本未加载",
      ],
      view: "timeline",
      viewLabel: "去看时间轴",
    },
  ];
  return nodes;
});

const activeTerminals = computed(() => terminal.sessions.length);
</script>

<template>
  <div class="workspace__view-inner" style="max-width: 760px">
    <div class="view-head">
      <div class="view-head__title">
        <div class="view-head__icon" style="background: var(--accent-soft)">🔗</div>
        <div>
          <h2>拓扑</h2>
          <div class="view-head__sub">
            钥匙链：每一环的状态灯都来自实测证据 · 点「下钻」跳到对应视图操作
          </div>
        </div>
      </div>
    </div>

    <div class="topo-chain">
      <div v-for="(n, i) in chain" :key="n.key" class="card info-card topo-node">
        <div v-if="i > 0" class="topo-link">↓</div>
        <div class="topo-node__head">
          <StatusLight :status="n.status" :size="10" />
          <b>{{ n.title }}</b>
          <NTag
            size="tiny"
            :bordered="false"
            :type="n.status === 'pass' ? 'success' : n.status === 'fail' ? 'error' : 'default'"
          >
            {{ { pending: "未开始", pass: "正常", warn: "有隐患", fail: "异常", running: "进行中", skip: "跳过" }[n.status] }}
          </NTag>
          <NButton v-if="n.view" size="tiny" quaternary style="margin-left: auto" @click="app.activeView = n.view">
            {{ n.viewLabel }} →
          </NButton>
        </div>
        <div class="topo-node__body mono">
          <div v-for="(l, j) in n.lines" :key="j" style="font-size: 12px; color: var(--text-2)">{{ l }}</div>
        </div>
      </div>
    </div>

    <div class="card info-card">
      <h3>旁路工具</h3>
      <div class="connect-row">
        <NButton size="small" secondary @click="app.activeView = 'terminal'">
          原始终端（{{ activeTerminals }} 个会话）
        </NButton>
        <NButton size="small" secondary @click="app.activeView = 'diff'">差分矩阵</NButton>
        <NButton size="small" secondary @click="app.activeView = 'ledger'">档案台账</NButton>
      </div>
      <p class="muted" style="margin: 8px 0 0; font-size: 11px">
        CHK-08 版本一致性：{{ versionCheck?.status ?? "未体检" }}
        <span v-if="versionCheck?.evidence?.length">—— {{ versionCheck.evidence[versionCheck.evidence.length - 1] }}</span>
      </p>
    </div>
  </div>
</template>

<style>
.topo-chain {
  display: flex;
  flex-direction: column;
  gap: 0;
}
.topo-node {
  position: relative;
}
.topo-link {
  text-align: center;
  color: var(--text-3);
  font-size: 16px;
  line-height: 1.2;
  margin: -4px 0;
}
.topo-node__head {
  display: flex;
  align-items: center;
  gap: 8px;
}
.topo-node__body {
  margin-top: 6px;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
</style>
