<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { NButton, NInput, NInputNumber, NSpin, NTag, useMessage } from "naive-ui";
import {
  RefreshOutline,
  FlashOutline,
  GitNetworkOutline,
  LinkOutline,
  AppsOutline,
  ArrowDownOutline,
  PulseOutline,
  EyeOutline,
  ConstructOutline,
  ArchiveOutline,
} from "@vicons/ionicons5";
import { usePipelineStore } from "@/stores/pipeline";
import { useSettingsStore } from "@/stores/settings";
import { api, type AdbDevice, type AdbResolveReport, type ConnectReport } from "@/api";
import { isTauri } from "@/utils/env";
import StatusLight from "@/components/StatusLight.vue";
import CheckItem from "@/components/CheckItem.vue";
import SessionConsole from "@/components/SessionConsole.vue";
import RecorderCard from "@/components/RecorderCard.vue";
import ProbeConsole from "@/components/ProbeConsole.vue";
import InjectionWizard from "@/components/InjectionWizard.vue";

/** 主视图：流水线（M0 = 环境体检 + 设备连接两个节点可用） */
const pipeline = usePipelineStore();
const settings = useSettingsStore();
const message = useMessage();

const activeNode = computed(() => pipeline.selectedNode);

const NODE_ICONS: Record<string, typeof PulseOutline> = {
  doctor: PulseOutline,
  connect: LinkOutline,
  load: AppsOutline,
  inject: ArrowDownOutline,
  probe: GitNetworkOutline,
  observe: EyeOutline,
  restore: ConstructOutline,
  archive: ArchiveOutline,
};
const nodeIcon = computed(() => NODE_ICONS[activeNode.value] ?? PulseOutline);

const MODE_LABEL: Record<string, string> = { quick: "快速体检", deep: "深度体检" };

// ---- 体检总览统计 ----
const stats = computed(() => {
  const checks = pipeline.doctor?.checks ?? [];
  return {
    pass: checks.filter((c) => c.status === "pass").length,
    warn: checks.filter((c) => c.status === "warn").length,
    fail: checks.filter((c) => c.status === "fail").length,
    total: checks.length,
    seconds: pipeline.doctor ? (pipeline.doctor.duration_ms / 1000).toFixed(1) : "0.0",
    mode: pipeline.doctor ? MODE_LABEL[pipeline.doctor.mode] ?? pipeline.doctor.mode : "",
  };
});

const OVERALL_LABEL: Record<string, string> = {
  pass: "可以干活",
  warn: "带警告，可用",
  fail: "有问题，先处理",
  skip: "未体检",
};

// ---- adb 解析与连接（节点2 设备连接） ----
const adbReport = ref<AdbResolveReport | null>(null);
const devices = ref<AdbDevice[]>([]);
const connectHost = ref("127.0.0.1");
const connectPort = ref<number>(16384);
const connecting = ref(false);
const lastConnect = ref<ConnectReport | null>(null);

const connectNode = computed(() => pipeline.nodes[1]);

async function refreshAdb() {
  try {
    adbReport.value = await api.resolveAdb();
    devices.value = await api.adbDevices();
  } catch (e) {
    message.error(String(e));
  }
}

async function doConnect(selfHeal: boolean) {
  connecting.value = true;
  try {
    lastConnect.value = selfHeal
      ? await api.adbSelfHeal(connectHost.value, connectPort.value)
      : await api.adbConnect(connectHost.value, connectPort.value);
    if (lastConnect.value.ok) {
      message.success(`已连接 ${lastConnect.value.serial}`);
      connectNode.value.status = "pass";
    } else {
      message.warning(`连接失败：${lastConnect.value.state}`);
      connectNode.value.status = "fail";
    }
    connectNode.value.summary = lastConnect.value.ok
      ? `已连接 ${lastConnect.value.serial}`
      : `连接失败（${lastConnect.value.attempts} 次尝试）`;
    await refreshAdb();
  } catch (e) {
    message.error(String(e));
    connectNode.value.status = "fail";
  } finally {
    connecting.value = false;
  }
}

onMounted(() => {
  // 浏览器预览（无 Tauri IPC）时跳过，避免一屏错误浮层
  if (!isTauri()) return;
  void refreshAdb();
  // 启动自动体检由设置控制（默认关，连接动作留给深度体检兜底）
  if (!pipeline.doctor && settings.config?.doctor.auto_run_on_startup) {
    void pipeline.runDoctor(false);
  }
});
</script>

<template>
  <div class="workspace__view-inner">
    <!-- ============ 节点1：环境体检 ============ -->
    <template v-if="activeNode === 'doctor'">
      <div class="view-head">
        <div class="view-head__title">
          <div class="view-head__icon"><PulseOutline size="18" /></div>
          <div>
            <h2>环境体检</h2>
            <div class="view-head__sub">
              动手前说清「这台机器能不能干活」（文档04-A，10 项检查）· 绿 = 有证据
            </div>
          </div>
        </div>
        <div class="view-head__actions">
          <NButton
            class="btn-hero"
            size="small"
            :loading="pipeline.doctorRunning"
            @click="pipeline.runDoctor(false)"
          >
            <template #icon><FlashOutline /></template>
            快速体检
          </NButton>
          <NButton
            size="small"
            secondary
            :loading="pipeline.doctorRunning"
            :title="'主动连接端口 + offline 自愈（每端口最长 15s，属慢操作兜底）'"
            @click="pipeline.runDoctor(true)"
          >
            <template #icon><RefreshOutline /></template>
            深度体检（连接重试）
          </NButton>
        </div>
      </div>

      <div v-if="pipeline.doctorError" class="doctor-error">
        体检执行失败：{{ pipeline.doctorError }}
      </div>

      <!-- 总览卡 -->
      <div v-if="pipeline.doctor" class="hero-card">
        <div class="hero-card__status">
          <span class="hero-card__status-label">
            {{ stats.mode }} · 总评
          </span>
          <span class="hero-card__status-value">
            <StatusLight :status="pipeline.doctor.overall" :size="13" />
            {{ OVERALL_LABEL[pipeline.doctor.overall] ?? pipeline.doctor.overall }}
          </span>
        </div>
        <div class="hero-card__stats">
          <div class="stat-block">
            <div class="stat-block__num stat-block__num--pass">{{ stats.pass }}<span style="color:var(--text-3);font-size:12px">/{{ stats.total }}</span></div>
            <div class="stat-block__label">通过</div>
          </div>
          <div class="stat-block">
            <div class="stat-block__num stat-block__num--warn">{{ stats.warn }}</div>
            <div class="stat-block__label">警告</div>
          </div>
          <div class="stat-block">
            <div class="stat-block__num stat-block__num--fail">{{ stats.fail }}</div>
            <div class="stat-block__label">失败</div>
          </div>
          <div class="stat-block">
            <div class="stat-block__num stat-block__num--plain">{{ stats.seconds }}s</div>
            <div class="stat-block__label">总耗时</div>
          </div>
        </div>
        <div class="hero-card__adb" :title="pipeline.doctor.adb_path">
          adb: {{ pipeline.doctor.adb_path || "未找到" }}
          <template v-if="pipeline.doctor.device_serial">
            <br />设备: {{ pipeline.doctor.device_serial }}
          </template>
        </div>
      </div>

      <NSpin :show="pipeline.doctorRunning">
        <div class="check-list">
          <CheckItem v-for="c in pipeline.doctor?.checks ?? []" :key="c.id" :check="c" />
          <div v-if="!pipeline.doctor && !pipeline.doctorRunning" class="placeholder-view" style="height: 240px">
            <span>尚未体检</span>
            <span class="placeholder-view__milestone">「快速体检」&lt;2s 出结果 · 连不上再点「深度体检」</span>
          </div>
        </div>
      </NSpin>
    </template>

    <!-- ============ 节点2：设备连接 ============ -->
    <template v-else-if="activeNode === 'connect'">
      <div class="view-head">
        <div class="view-head__title">
          <div class="view-head__icon"><LinkOutline size="18" /></div>
          <div>
            <h2>设备连接</h2>
            <div class="view-head__sub">
              connect 必带 15s 硬超时（E-02）· offline 自愈曲线（E-03）· 假绿灯二次确认（S-05）
            </div>
          </div>
        </div>
        <div class="view-head__actions">
          <NButton size="small" secondary @click="refreshAdb">
            <template #icon><RefreshOutline /></template>
            刷新
          </NButton>
        </div>
      </div>

      <div class="card connect-card">
        <h3>adb 连接</h3>
        <div class="connect-row">
          <NInput v-model:value="connectHost" size="small" style="width: 180px" placeholder="127.0.0.1" />
          <NInputNumber v-model:value="connectPort" size="small" style="width: 120px" :show-button="false" placeholder="16384" />
          <NButton size="small" type="primary" :loading="connecting" @click="doConnect(false)">
            连接
          </NButton>
          <NButton size="small" secondary :loading="connecting" title="disconnect → 2s → connect，×5（E-03）" @click="doConnect(true)">
            自愈重连
          </NButton>
        </div>
        <div v-if="lastConnect" class="connect-report">
          <span :class="['status-chip', lastConnect.ok ? 'status-chip--pass' : 'status-chip--fail']">
            {{ lastConnect.ok ? `已连接 ${lastConnect.serial}` : `失败：${lastConnect.state}（${lastConnect.attempts} 次）` }}
          </span>
          <pre class="check-item__evidence">{{ lastConnect.evidence.join("\n") }}</pre>
        </div>
      </div>

      <div class="card info-card">
        <h3>当前设备列表（adb devices）</h3>
        <table class="plain-table">
          <thead>
            <tr><th>serial</th><th>state</th><th>model</th></tr>
          </thead>
          <tbody>
            <tr v-for="d in devices" :key="d.serial">
              <td class="mono">{{ d.serial }}</td>
              <td>
                <span :class="['status-chip', d.state === 'device' ? 'status-chip--pass' : 'status-chip--warn']">
                  {{ d.state }}
                </span>
              </td>
              <td>{{ d.model ?? "—" }}</td>
            </tr>
            <tr v-if="devices.length === 0">
              <td colspan="3" class="muted">无设备。启动 MuMu 后用上方按钮连接（端口列表可在设置中配置）。</td>
            </tr>
          </tbody>
        </table>
      </div>

      <div class="card info-card">
        <h3>adb 候选探测</h3>
        <table class="plain-table">
          <thead>
            <tr><th>路径</th><th>来源</th><th>状态</th></tr>
          </thead>
          <tbody>
            <tr v-for="c in adbReport?.candidates ?? []" :key="c.path">
              <td class="mono">{{ c.path }}</td>
              <td>{{ c.source }}</td>
              <td>
                <NTag v-if="c.chosen" size="small" type="info" :bordered="false">已选用</NTag>
                <span v-else-if="c.exists" class="status-chip status-chip--pass">存在</span>
                <span v-else class="muted">不存在</span>
              </td>
            </tr>
            <tr v-if="!adbReport">
              <td colspan="3" class="muted">加载中…</td>
            </tr>
          </tbody>
        </table>
        <p class="muted" style="margin: 10px 0 0">
          规则（E-04）：模拟器自带 adb 优先 → 设置里的自定义路径 → 项目 bin\adb → PATH。
        </p>
      </div>

      <!-- ============ 会话控制台（frida-server / forward / attach） ============ -->
      <SessionConsole />

      <!-- ============ 操作记录（Recorder v1） ============ -->
      <RecorderCard />
    </template>

    <!-- ============ 节点5：探针注入（M2） ============ -->
    <template v-else-if="activeNode === 'probe'">
      <div class="view-head">
        <div class="view-head__title">
          <div class="view-head__icon"><GitNetworkOutline size="18" /></div>
          <div>
            <h2>探针注入</h2>
            <div class="view-head__sub">
              填表挂钩（零行 JS）· 探索器找目标 · REPL 直接求值 · 命中进时间轴
            </div>
          </div>
        </div>
      </div>
      <ProbeConsole />
    </template>

    <!-- ============ 节点4：数据回灌（M3） ============ -->
    <template v-else-if="activeNode === 'inject'">
      <div class="view-head">
        <div class="view-head__title">
          <div class="view-head__icon"><ArrowDownOutline size="18" /></div>
          <div>
            <h2>数据回灌</h2>
            <div class="view-head__sub">
              七步逐条状态灯 · 停应用/空跑建档不可跳过（D-06/D-07）· 推送失败自动降级中转（D-01）· 登录态预警（O-03）
            </div>
          </div>
        </div>
      </div>
      <InjectionWizard />
    </template>

    <!-- ============ 其余节点：里程碑占位 ============ -->
    <template v-else>
      <div class="view-head">
        <div class="view-head__title">
          <div class="view-head__icon">
            <component :is="nodeIcon" size="18" />
          </div>
          <div>
            <h2>{{ pipeline.nodes.find((n) => n.key === activeNode)?.name }}</h2>
          </div>
        </div>
      </div>
      <div class="placeholder-view" style="height: 50vh">
        <span>该节点在后续里程碑交付</span>
        <span class="placeholder-view__milestone">
          应用装载/数据回灌 → M1/M3 · 探针注入 → M2 · 观测 → M2 · 还原 → M4 · 归档 → M5
        </span>
      </div>
    </template>
  </div>
</template>

<style>
.doctor-error {
  border: 1px solid var(--st-fail);
  color: var(--st-fail);
  border-radius: var(--radius-md);
  padding: 10px 14px;
  margin-bottom: 12px;
  background: var(--st-fail-soft);
}
</style>
