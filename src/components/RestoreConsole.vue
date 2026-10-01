<script setup lang="ts">
import { computed, reactive, ref } from "vue";
import { NButton, NInput, NTag, useMessage } from "naive-ui";
import { FlashOutline, PlayOutline, SearchOutline } from "@vicons/ionicons5";
import {
  api,
  type BruteEstimate,
  type BruteResult,
  type CryptoReconstructResult,
  type CryptoSample,
} from "@/api";
import StatusLight from "@/components/StatusLight.vue";

/** 还原节点（M4）：算法还原（穷举+双样本防假命中）+ 爆破编排（预估/内置/C 骨架/hashcat） */
const message = useMessage();

// ---------- 算法还原 ----------
const samples = reactive<CryptoSample[]>([
  { plaintext: "", salt: "", target: "" },
  { plaintext: "", salt: "", target: "" },
]);
const recRunning = ref(false);
const recResult = ref<CryptoReconstructResult | null>(null);

async function onReconstruct() {
  const valid = samples.filter((s) => s.plaintext.trim() && s.target.trim());
  if (valid.length === 0) {
    message.warning("至少填一组 (明文, 目标值)；两组样本可防假命中");
    return;
  }
  recRunning.value = true;
  recResult.value = null;
  try {
    recResult.value = await api.cryptoReconstruct(valid);
    if (recResult.value.error) {
      message.error(recResult.value.error);
    } else {
      message.success(`算法定死（${recResult.value.candidates.length} 个候选，双样本自证 ${recResult.value.selfTestPassed ? "通过" : "未通过"}）`);
    }
  } catch (e) {
    message.error(String(e));
  } finally {
    recRunning.value = false;
  }
}

// ---------- 爆破编排 ----------
const brute = reactive({
  mask: "?d?d?d?d",
  salt: "",
  selftestPwd: "",
  maxCandidates: 5_000_000,
});
const est = ref<BruteEstimate | null>(null);
const estLoading = ref(false);
const bruteRunning = ref(false);
const bruteResult = ref<BruteResult | null>(null);
const cPath = ref<string | null>(null);

const schemeForBrute = computed(() => recResult.value?.scheme ?? null);
const needScheme = computed(() => !schemeForBrute.value);

async function onEstimate() {
  if (!schemeForBrute.value) return;
  estLoading.value = true;
  try {
    est.value = await api.bruteEstimate(schemeForBrute.value, brute.mask.trim());
  } catch (e) {
    message.error(String(e));
  } finally {
    estLoading.value = false;
  }
}

async function onRunBrute() {
  if (!schemeForBrute.value || !brute.salt.trim() || !brute.selftestPwd.trim() || !brute.mask.trim()) {
    message.warning("需要：还原出的方案 + 盐 + 自测明文（必须与还原时填的明文一致）+ 掩码");
    return;
  }
  bruteRunning.value = true;
  bruteResult.value = null;
  try {
    const known: CryptoSample = {
      plaintext: brute.selftestPwd.trim(),
      salt: brute.salt.trim(),
      target: samples[0].target.trim(),
    };
    bruteResult.value = await api.bruteRun(schemeForBrute.value, brute.mask.trim(), brute.salt.trim(), known, brute.maxCandidates);
    if (bruteResult.value.hit) message.success(`HIT pwd=${bruteResult.value.hit}`);
    else if (!bruteResult.value.self_test_passed) message.error(bruteResult.value.note);
    else message.warning(`未命中（${bruteResult.value.tried} 候选）`);
  } catch (e) {
    message.error(String(e));
  } finally {
    bruteRunning.value = false;
  }
}

async function onGenC() {
  if (!schemeForBrute.value || !samples[0].plaintext.trim()) return;
  try {
    const r = await api.bruteGenerateC(schemeForBrute.value, {
      plaintext: samples[0].plaintext.trim(),
      salt: brute.salt.trim(),
      target: samples[0].target.trim(),
    });
    cPath.value = r.path;
    message.success(`C 骨架已生成 → ${r.path}`);
  } catch (e) {
    message.error(String(e));
  }
}
</script>

<template>
  <!-- ============ 算法还原 ============ -->
  <div class="card info-card">
    <h3>算法还原（穷举：哈希族 × 拼接 × 盐形态 × 链式 × 迭代 1~100 万 × 编码）</h3>
    <table class="plain-table">
      <thead><tr><th style="width: 120px">样本</th><th>明文</th><th>盐（如库中 base64 原文）</th><th>目标值（库中存储值）</th></tr></thead>
      <tbody>
        <tr v-for="(s, i) in samples" :key="i">
          <td><NTag size="small" :bordered="false" :type="i === 0 ? 'info' : 'success'">样本{{ i + 1 }}{{ i > 0 ? "（防假命中）" : "" }}</NTag></td>
          <td><NInput v-model:value="s.plaintext" size="small" placeholder="用户输入的明文密码" /></td>
          <td><NInput v-model:value="s.salt" size="small" placeholder= /></td>
          <td><NInput v-model:value="s.target" size="small" placeholder="Base64 或 hex 编码的哈希值" /></td>
        </tr>
      </tbody>
    </table>
    <div class="connect-row" style="margin-top: 10px">
      <NButton class="btn-hero" size="small" :loading="recRunning" @click="onReconstruct">
        <template #icon><SearchOutline /></template>
        运行还原（穷举扫描）
      </NButton>
      <span class="muted">两组样本不同明文 = 防假命中（文档04-F）；扫描最长约几分钟</span>
    </div>

    <!-- 方案结果 -->
    <div v-if="recResult && !recResult.error" class="scheme-result">
      <div class="scheme-line">
        <StatusLight :status="recResult.selfTestPassed ? 'pass' : 'warn'" show-text />
        <b class="mono">{{ recResult.humanDesc }}</b>
      </div>
      <div class="scheme-line" v-if="recResult.hashcatMode">
        <NTag size="small" type="success" :bordered="false">hashcat 有模式</NTag>
        <code class="mono" style="font-size: 11px">{{ recResult.hashcatCmd }}</code>
      </div>
      <div class="scheme-line" v-else>
        <NTag size="small" type="warning" :bordered="false">无现成 hashcat 模式</NTag>
        <span class="muted" style="font-size: 12px">→ 用下方「生成 C 骨架」（writeup 路线）</span>
      </div>
      <div v-if="recResult.candidates.length > 1" class="scheme-line muted" style="font-size: 11px">
        另有 {{ recResult.candidates.length - 1 }} 个同真候选（防过度泛化，全列出，R5）
      </div>
      <details>
        <summary class="muted" style="cursor: pointer; font-size: 12px">Python 验证骨架（规格书）</summary>
        <pre class="check-item__evidence">{{ recResult.pythonSkeleton }}</pre>
      </details>
    </div>
    <div v-if="recResult?.error" class="doctor-error">{{ recResult.error }}</div>
  </div>

  <!-- ============ 爆破编排 ============ -->
  <div class="card info-card" :style="needScheme ? 'opacity: 0.55' : ''">
    <h3>爆破编排（★自测不过不许全量，C-07）</h3>
    <div v-if="needScheme" class="muted" style="margin-bottom: 8px">先运行算法还原，拿到方案后这里自动可用。</div>
    <template v-else>
      <div class="connect-row" style="margin-bottom: 8px">
        <NInput v-model:value="brute.mask" size="small" placeholder="掩码，如 ?u?l?l?d?d?d?d?d?d" style="width: 260px" />
        <NInput v-model:value="brute.salt" size="small" placeholder="盐（同还原时）" style="width: 220px" />
        <NInput v-model:value="brute.selftestPwd" size="small" placeholder="自测明文（= 还原时的明文）" style="width: 220px" />
      </div>
      <div class="connect-row">
        <NButton size="small" secondary :loading="estLoading" @click="onEstimate">
          <template #icon><FlashOutline /></template>
          预估（总数/速率/ETA）
        </NButton>
        <NButton size="small" type="primary" :loading="bruteRunning" title="内置爆破（Rust，小空间直接跑）" @click="onRunBrute">
          <template #icon><PlayOutline /></template>
          内置爆破（自测→全量）
        </NButton>
        <NButton size="small" secondary @click="onGenC">生成 C 骨架</NButton>
      </div>

      <div v-if="est" class="scheme-line" style="margin-top: 10px">
        <NTag size="small" :bordered="false" :type="est.engine === 'builtin' ? 'success' : 'warning'">
          推荐：{{ est.engine === "builtin" ? "内置爆破" : est.engine === "hashcat" ? "hashcat" : "生成 C 骨架" }}
        </NTag>
        <span class="mono" style="font-size: 12px">
          总数 {{ est.total.toLocaleString() }} · 预估 {{ est.est_speed.toLocaleString() }}/s · ETA {{ est.eta_seconds.toLocaleString() }}s
        </span>
        <div class="muted" style="font-size: 11px">{{ est.reason }}</div>
      </div>

      <div v-if="bruteResult" class="scheme-line" style="margin-top: 10px">
        <template v-if="bruteResult.hit">
          <NTag size="small" type="success">HIT</NTag>
          <b class="mono" style="font-size: 15px; color: var(--st-pass)">pwd={{ bruteResult.hit }}</b>
          <span class="muted" style="font-size: 11px">{{ bruteResult.tried }} 候选 / {{ bruteResult.duration_ms }}ms（记得反向校验：真机验证，双证据闭环）</span>
        </template>
        <template v-else-if="!bruteResult.self_test_passed">
          <NTag size="small" type="error">★自测失败</NTag>
          <span style="font-size: 12px">{{ bruteResult.note }}</span>
        </template>
        <template v-else>
          <NTag size="small" type="warning">未命中</NTag>
          <span style="font-size: 12px">{{ bruteResult.note }}</span>
        </template>
      </div>
      <div v-if="cPath" class="muted mono" style="font-size: 11px; margin-top: 6px">C 骨架：{{ cPath }}（gcc -O3 -march=native -fopenmp 编译；运行先过自测桩）</div>
    </template>
  </div>
</template>

<style>
.scheme-result {
  margin-top: 12px;
  padding: 12px 14px;
  background: var(--st-pass-soft);
  border: 1px solid var(--border-1);
  border-radius: var(--radius-md);
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.scheme-line {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}
</style>
