<script setup lang="ts">
import { computed, onMounted, reactive, ref } from "vue";
import { NButton, NInput, NInputNumber, NTag, useMessage } from "naive-ui";
import { SaveOutline, TrashOutline } from "@vicons/ionicons5";
import { api, type AppProfileRow } from "@/api";

/** AppProfile（U9：一次录好复用）——数据回灌节点的档案编辑器 */
const message = useMessage();

const caseName = computed(() => "默认案");
const profiles = ref<AppProfileRow[]>([]);
const editingId = ref<number | null>(null);

const form = reactive({
  package: "",
  uid: null as number | null,
  apkPath: "",
  dataDirs: "/data/data/<pkg>/databases",
  secretFiles: "",
  secretTransform: "",
  entryGesture: "",
  entryCoords: "",
  probeTargets: "",
  notes: "",
});

function loadRow(row: Partial<AppProfileRow> & Record<string, unknown>) {
  editingId.value = row.id ?? null;
  form.package = String(row.package ?? "");
  form.uid = typeof row.uid === "number" ? row.uid : null;
  form.apkPath = String(row.apk_path ?? "");
  form.dataDirs = String(row.data_dirs ?? "");
  form.secretFiles = String(row.secret_files ?? "");
  form.secretTransform = String(row.secret_transform ?? "");
  form.entryGesture = String(row.entry_gesture ?? "");
  form.entryCoords = String(row.entry_coords ?? "");
  form.probeTargets = String(row.probe_targets ?? "");
  form.notes = String(row.notes ?? "");
}

function resetForm() {
  editingId.value = null;
  form.package = "";
  form.uid = null;
  form.apkPath = "";
  form.dataDirs = "/data/data/<pkg>/databases";
  form.secretFiles = "";
  form.secretTransform = "";
  form.entryGesture = "";
  form.entryCoords = "";
  form.probeTargets = "";
  form.notes = "";
}

async function refresh() {
  profiles.value = await api.profileList(caseName.value);
}

async function onSave() {
  try {
    await api.profileSave({
      caseName: caseName.value,
      id: editingId.value ?? undefined,
      package: form.package.trim(),
      uid: form.uid,
      apkPath: form.apkPath.trim(),
      dataDirs: form.dataDirs.trim(),
      secretFiles: form.secretFiles.trim(),
      secretTransform: form.secretTransform.trim(),
      entryGesture: form.entryGesture.trim(),
      entryCoords: form.entryCoords.trim(),
      probeTargets: form.probeTargets.trim(),
      notes: form.notes.trim(),
    });
    message.success("档案已保存（第二个同类 App 的起点，U9）");
    await refresh();
  } catch (e) {
    message.error(String(e));
  }
}

async function onDelete(id: number) {
  await api.profileDelete(id);
  await refresh();
}

/** 把档案的包名/目录带入回灌向导（通过事件总线通知父组件） */
const emit = defineEmits<{ (e: "apply", pkg: string, dir: string): void }>();

function applyToWizard(row: Partial<AppProfileRow> & Record<string, unknown>) {
  const firstDir = String(row.data_dirs ?? "").split(",")[0]?.trim() ?? "";
  emit("apply", String(row.package ?? ""), firstDir);
  message.success(`已带入回灌配置：${row.package}`);
}

onMounted(refresh);
</script>

<template>
  <div class="card info-card">
    <h3>
      AppProfile（目标档案）
      <NTag size="small" :bordered="false">U9：一次录好复用</NTag>
    </h3>
    <div class="connect-row" style="margin-bottom: 8px; flex-wrap: wrap">
      <NInput v-model:value="form.package" size="small" placeholder="包名 *" style="width: 240px" />
      <NInputNumber v-model:value="form.uid" size="small" placeholder="uid（实测）" :show-button="false" style="width: 120px" />
      <NInput v-model:value="form.apkPath" size="small" placeholder="APK 路径（只读）" style="flex: 1" />
    </div>
    <div class="connect-row" style="margin-bottom: 8px; flex-wrap: wrap">
      <NInput v-model:value="form.dataDirs" size="small" placeholder="数据目录（逗号分隔）" style="flex: 1" />
      <NInput v-model:value="form.secretFiles" size="small" placeholder="口令/密钥文件（如 app_flutter/files/password.json）" style="flex: 1" />
    </div>
    <div class="connect-row" style="margin-bottom: 8px; flex-wrap: wrap">
      <NInput v-model:value="form.secretTransform" size="small" placeholder="口令变换（如 [1:-2]）" style="width: 180px" />
      <NInput v-model:value="form.entryGesture" size="small" placeholder="入口手势（如 长按齿轮1.2s）" style="width: 220px" />
      <NInput v-model:value="form.entryCoords" size="small" placeholder="关键坐标（JSON）" style="flex: 1" />
    </div>
    <div class="connect-row" style="margin-bottom: 8px; flex-wrap: wrap">
      <NInput v-model:value="form.probeTargets" size="small" placeholder="探针目标（如 AESUtil.hashPassword）" style="flex: 1" />
      <NInput v-model:value="form.notes" size="small" placeholder="备注" style="flex: 1" />
    </div>
    <div class="connect-row">
      <NButton class="btn-hero" size="small" @click="onSave">
        <template #icon><SaveOutline /></template>
        {{ editingId ? "更新档案" : "新建档案" }}
      </NButton>
      <NButton v-if="editingId" size="small" quaternary @click="resetForm">取消编辑</NButton>
    </div>

    <table v-if="profiles.length" class="plain-table" style="margin-top: 12px">
      <thead><tr><th>包名</th><th>uid</th><th>数据目录</th><th>口令变换</th><th style="width: 120px" /></tr></thead>
      <tbody>
        <tr v-for="p in profiles" :key="p.id">
          <td class="mono" style="font-size: 12px">{{ p.package }}</td>
          <td class="mono">{{ p.uid ?? "—" }}</td>
          <td style="font-size: 11px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 240px">{{ p.data_dirs }}</td>
          <td class="mono" style="font-size: 11px">{{ p.secret_transform || "—" }}</td>
          <td>
            <NButton size="tiny" quaternary @click="loadRow(p)">编辑</NButton>
            <NButton size="tiny" quaternary type="primary" @click="applyToWizard(p)">带入回灌</NButton>
            <NButton size="tiny" quaternary type="warning" @click="onDelete(p.id)"><TrashOutline /></NButton>
          </td>
        </tr>
      </tbody>
    </table>
    <p v-else class="muted" style="margin: 10px 0 0; font-size: 11px">
      档案是「知识资产」：包名/uid/数据目录/口令文件/变换规则/入口手势——第二个同类 App 的起点。
    </p>
  </div>
</template>
