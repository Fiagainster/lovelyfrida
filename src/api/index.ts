/**
 * 前端 API 层：所有 invoke 集中于此，类型与 Rust 侧 struct 手工镜像（标注对应关系）。
 * M1 引入 tauri-specta 自动生成后，此文件退化为 re-export。
 */
import { isTauri } from "@/utils/env";

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) {
    throw new Error(`浏览器预览模式：命令 ${cmd} 不可用（需在 Tauri 运行时内）`);
  }
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(cmd, args);
}

/** 错误归一化展示（批次⑫）：Rust 的 Err(String) 本身就是面向用户的文案，直接用；
 *  JS Error 取 message；其余 JSON 化兜底。展示格式不再时而是 "Error: x" 时而是对象 dump。
 *  全局红色 fatal 条（main.ts 兜底）只留给真崩溃——常规操作失败一律 message.error(errMsg(e))。 */
export function errMsg(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  try {
    return JSON.stringify(e);
  } catch {
    return String(e);
  }
}

// ---------- 类型（对应 Rust struct） ----------

/** 六态状态（文档03：○/◐/●/◑/✖/⊘，Rust 侧为 String，此约束只在前端） */
export type CheckStatus = "pending" | "running" | "pass" | "warn" | "fail" | "skip";

/** 对应 Rust CheckResult */
export interface CheckResult {
  id: string;
  name: string;
  status: CheckStatus;
  evidence: string[];
  fix: string | null;
  rule: string | null;
  command: string | null;
  duration_ms: number;
}

/** 对应 Rust DoctorReport */
export interface DoctorReport {
  checks: CheckResult[];
  overall: CheckStatus;
  mode: "quick" | "deep";
  adb_path: string;
  adb_source: string;
  device_serial: string | null;
  duration_ms: number;
}

/** 对应 Rust UiConfig */
export interface UiConfig {
  theme: "dark" | "light";
  language: string;
  nav_expanded: boolean;
}

/** 对应 Rust DoctorConfig（体检可配置项） */
export interface DoctorConfig {
  auto_run_on_startup: boolean;
  emulator_ports: number[];
  adb_extra_paths: string[];
  deep_connect_timeout_s: number;
}

/** 对应 Rust AppConfig（文档06 config.toml） */
export interface AppConfig {
  workspace_root: string;
  cases_root: string;
  adb_path: string;
  preferred_channel: "auto" | "a" | "b" | "c";
  frida_port: number;
  adb_connect_timeout_s: number;
  read_only_roots: string[];
  mask_secrets_in_logs: boolean;
  retain_raw_log_lines: number;
  ui: UiConfig;
  doctor: DoctorConfig;
}

export interface FirstRunItem {
  id: string;
  name: string;
  ok: boolean;
  detail: string;
}

export interface FirstRunReport {
  ok: boolean;
  items: FirstRunItem[];
}

export interface GuardVerdict {
  allowed: boolean;
  reason: string;
  normalized_path: string;
}

export interface AdbDevice {
  serial: string;
  state: string;
  model: string | null;
}

export interface ConnectReport {
  ok: boolean;
  serial: string;
  state: string;
  attempts: number;
  evidence: string[];
  elapsed_ms: number;
}

export interface AppInfo {
  version: string;
  root: string;
  workspace_dir: string;
  cases_dir: string;
  logs_dir: string;
}

export interface AdbCandidate {
  path: string;
  exists: boolean;
  chosen: boolean;
  source: string;
}

export interface AdbResolveReport {
  path: string | null;
  source: string;
  candidates: AdbCandidate[];
}

// ---------- frida 会话（M1 通道B） ----------

export interface ServerStatusReport {
  client_version: string | null;
  client_error: string | null;
  device_serial: string | null;
  device_server_present: boolean | null;
  device_server_version: string | null;
  server_running: boolean | null;
  forward_established: boolean | null;
  matrix: string[];
  port: number;
  overall: CheckStatus;
}

/** frida-server 按需下载报告（C1：manifest 驱动 + sha256 对账后落工作区） */
export interface ServerFetchReport {
  version: string;
  entries: { abi: string; path: string; size: number; skipped: boolean; error: string | null }[];
  elapsed_ms: number;
}

export interface StepReport {
  name: string;
  status: "pass" | "warn" | "fail";
  evidence: string[];
}

export interface ProcEntry {
  pid: number;
  name: string;
  group: "system" | "user";
  running: boolean;
}

export type SessionPhase =
  | "idle"
  | "discovering"
  | "device_ready"
  | "server_up"
  | "forwarded"
  | "attached"
  | "running"
  | "failed"
  | "stopped";

export interface SessionSnapshot {
  phase: SessionPhase;
  evidence: string[];
  device: string | null;
  /** forward 的主机侧端口（S-06：与设备端口不一致时由探测得出） */
  forward_host_port?: number | null;
  target: string | null;
  session_id: number | null;
  script_id: number | null;
  hello: Record<string, unknown> | null;
  channel: string;
  updated_at: string;
  /** cases.db sessions 行 id（None=落库跳过） */
  db_session_id?: number | null;
}

/** 终端 */
export interface TerminalInfo {
  id: number;
  serial: string;
  created_at: string;
}

/** Recorder 步骤（文档03§五） */
export interface RecordedStep {
  seq: number;
  ts: string;
  action: string;
  command: string;
  params: Record<string, unknown>;
  result: string;
  duration_ms: number;
}

export interface ExportResult {
  path: string;
  count: number;
}

/** Rust 事件：frida-event（FridaEvent serde tag=event） */
export interface FridaEventPayload {
  event: "ready" | "message" | "detached" | "device_lost" | "spawn_added";
  params: Record<string, unknown>;
}

// ---------- 命令封装 ----------

export const api = {
  getAppInfo: () => invoke<AppInfo>("get_app_info"),
  getConfig: () => invoke<AppConfig>("get_config"),
  updateConfig: (config: AppConfig) => invoke<AppConfig>("update_config", { config }),

  resolveAdb: () => invoke<AdbResolveReport>("resolve_adb"),
  adbDevices: () => invoke<AdbDevice[]>("adb_devices"),
  adbConnect: (host: string, port: number) =>
    invoke<ConnectReport>("adb_connect", { host, port }),
  adbSelfHeal: (host: string, port: number) =>
    invoke<ConnectReport>("adb_self_heal", { host, port }),

  doctorRun: (deep: boolean) => invoke<DoctorReport>("doctor_run", { deep }),
  firstRunSelfCheck: () => invoke<FirstRunReport>("first_run_self_check"),
  guardCheckWrite: (path: string) => invoke<GuardVerdict>("guard_check_write", { path }),

  // frida 会话（M1 通道B）
  fridaServerStatus: () => invoke<ServerStatusReport>("frida_server_status"),
  fridaServerInstall: () => invoke<StepReport[]>("frida_server_install"),
  fridaServerFetch: (version?: string | null, abis?: string[] | null) =>
    invoke<ServerFetchReport>("frida_server_fetch", { version: version ?? null, abis: abis ?? null }),
  fridaForwardSetup: () => invoke<StepReport>("frida_forward_setup"),
  fridaProcesses: () => invoke<ProcEntry[]>("frida_processes"),
  fridaSessionAttach: (target: number | string, caseName?: string) =>
    invoke<SessionSnapshot>("frida_session_attach", { target, caseName }),
  fridaSessionDetach: () => invoke<SessionSnapshot>("frida_session_detach"),
  fridaSessionStatus: () => invoke<SessionSnapshot>("frida_session_status"),
  fridaSessionPing: () => invoke<Record<string, unknown>>("frida_session_ping"),

  confirmClose: () => invoke<void>("confirm_close"),
  cancelClose: () => invoke<void>("cancel_close"),

  // 终端（PTY）
  terminalCreate: (serial: string) => invoke<TerminalInfo>("terminal_create", { serial }),
  terminalWrite: (id: number, data: string) => invoke<void>("terminal_write", { id, data }),
  terminalResize: (id: number, cols: number, rows: number) =>
    invoke<void>("terminal_resize", { id, cols, rows }),
  terminalClose: (id: number) => invoke<void>("terminal_close", { id }),

  // M4：算法还原 + 爆破（caseName 用于方案/作业落库）
  cryptoReconstruct: (samples: { plaintext: string; salt: string; target: string }[], caseName?: string) =>
    invoke<CryptoReconstructResult>("crypto_reconstruct", { samples, caseName }),
  bruteEstimate: (scheme: CryptoScheme, mask: string) =>
    invoke<BruteEstimate>("brute_estimate", { scheme, mask }),
  bruteRun: (scheme: CryptoScheme, mask: string, salt: string, known: CryptoSample, maxCandidates?: number, caseName?: string) =>
    invoke<BruteResult>("brute_run", { scheme, mask, salt, known, maxCandidates, caseName }),
  bruteGenerateC: (scheme: CryptoScheme, sample: CryptoSample, mask: string, caseName?: string) =>
    invoke<{ path: string }>("brute_generate_c", { scheme, sample, mask, caseName }),

  // M5：台账
  ledgerAdd: (payload: {
    caseName: string; questionId: string; question: string; answer: string;
    confidence: string; evidence: { kind: string; note: string }[];
    source: string; screenshotSlot: string;
  }) => invoke<number>("ledger_add", { ...payload, caseName: payload.caseName }),
  ledgerList: (caseName: string) => invoke<Finding[]>("ledger_list", { caseName }),
  ledgerDelete: (id: number) => invoke<void>("ledger_delete", { id }),
  ledgerExportMd: (caseName: string) => invoke<string>("ledger_export_md", { caseName }),
  ledgerExportBundle: (caseName: string) => invoke<string>("ledger_export_bundle", { caseName }),

  // 能力包 A/B/D
  scriptList: () => invoke<ScriptInfo[]>("script_list"),
  scriptRead: (name: string) => invoke<string>("script_read", { name }),
  scriptSave: (name: string, content: string) => invoke<string>("script_save", { name, content }),
  scriptDelete: (name: string) => invoke<void>("script_delete", { name }),
  profileSave: (p: ProfilePayload) => invoke<number>("profile_save", p),
  profileList: (caseName: string) => invoke<AppProfileRow[]>("profile_list", { caseName }),
  profileDelete: (id: number) => invoke<void>("profile_delete", { id }),
  dumpsList: () => invoke<{ name: string; size: number }[]>("dumps_list"),

  // B2 落库数据面：历史查询
  historySessions: (limit = 20) => invoke<HistorySession[]>("history_sessions", { limit }),
  historyRuns: (limit = 20) => invoke<HistoryRun[]>("history_runs", { limit }),
  historyExperiments: (limit = 20) => invoke<HistoryExperiment[]>("history_experiments", { limit }),
  historyBruteJobs: (limit = 20) => invoke<HistoryBruteJob[]>("history_brute_jobs", { limit }),

  // M3：回灌 + 实验
  injectionRun: (package_: string, files: { localPath: string; deviceDir: string; deviceName: string }[]) =>
    invoke<InjectionReport>("injection_run", { package: package_, files }),
  experimentRun: (exp: ExperimentConfig, caseName?: string) => invoke<ExperimentReport>("experiment_run", { exp, caseName }),

  // M2：agent RPC（探索器/探针/内存/REPL）
  fridaRpc: (f: string, args: unknown[]) => invoke<unknown>("frida_rpc", { f, args }),

    // Recorder
  recorderList: () => invoke<RecordedStep[]>("recorder_list"),
  recorderExport: (format: "ps1" | "sh" | "md" | "json") =>
    invoke<ExportResult>("recorder_export", { format }),
  recorderClear: () => invoke<void>("recorder_clear"),
};

// ---------- M2：探针 / trace ----------

/** 探针声明（对应 agent ProbeDecl） */
export interface ProbeDecl {
  id: string;
  clazz: string;
  method: string;
  maxLen?: number;
  captureRet?: boolean;
  backtrace?: boolean;
  condition?: string;
}

export interface ProbeStat {
  id: string;
  clazz: string;
  method: string;
  status: "active" | "waiting" | "error";
  hits: number;
  errors: number;
  lastError: string | null;
}

/** trace 事件（Rust TraceRecord） */
export interface TraceRecord {
  seq: number;
  wall: string;
  run_id: string;
  payload: Record<string, unknown>;
}

// ---------- M3：回灌 / 实验 ----------

export interface InjectionFile {
  localPath: string;
  deviceDir: string;
  deviceName: string;
}

export interface InjectionStep {
  name: string;
  status: "pass" | "warn" | "fail" | "skip";
  evidence: string[];
}

export interface InjectionReport {
  steps: InjectionStep[];
  login_state_warnings: string[];
  overall: "pass" | "warn" | "fail";
}

export interface ExperimentTemplate {
  name: string;
  content: string;
}

export interface ExperimentProbe {
  clazz: string;
  method: string;
  maxLen?: number;
  captureRet?: boolean;
}

export interface ExperimentConfig {
  package: string;
  deviceFileDir: string;
  deviceFileName: string;
  probe: ExperimentProbe;
  waitS: number;
  templates: ExperimentTemplate[];
}

export interface Observation {
  group: string;
  hits: number;
  args: { k: string; v: string }[] | null;
  ret: { k: string; v: string } | null;
  wall: string;
}

export interface ExperimentReport {
  experiment_id: string;
  observations: Observation[];
  errors: string[];
}

// ---------- M4：算法还原 / 爆破 ----------

export interface CryptoSample {
  plaintext: string;
  salt: string;
  target: string;
}

export interface CryptoScheme {
  family: string;
  concat: string;
  saltForm: string;
  chainInput: string;
  iterations: number;
  outputEncoding: string;
}

export interface CryptoReconstructResult {
  scheme: CryptoScheme | null;
  candidates: CryptoScheme[];
  selfTestPassed: boolean;
  humanDesc: string;
  pythonSkeleton: string;
  hashcatMode: string | null;
  hashcatCmd: string | null;
  error: string | null;
}

export interface BruteEstimate {
  total: number;
  est_speed: number;
  eta_seconds: number;
  engine: "builtin" | "hashcat" | "generate_c";
  reason: string;
}

export interface BruteResult {
  self_test_passed: boolean;
  hit: string | null;
  tried: number;
  duration_ms: number;
  note: string;
}

export interface Finding {
  id: number;
  case_id: number;
  question_id: string;
  question: string;
  answer: string;
  confidence: "high" | "medium" | "low";
  evidence: { kind: "math" | "device"; note: string }[];
  source: string;
  screenshot_slot: string;
}

// ---------- 能力包 ----------

export interface ScriptInfo {
  name: string;
  size: number;
  modified: string;
}

export interface ProfilePayload extends Record<string, unknown> {
  caseName: string;
  id?: number;
  package: string;
  uid?: number | null;
  apkPath: string;
  dataDirs: string;
  secretFiles: string;
  secretTransform: string;
  entryGesture: string;
  entryCoords: string;
  probeTargets: string;
  notes: string;
}

export interface AppProfileRow extends ProfilePayload {
  id: number;
}

// ---------- B2 落库数据面：历史行 ----------

export interface HistorySession {
  id: number;
  case_name: string;
  device_serial: string | null;
  target: string;
  channel: string;
  state: string;
  started_at: string;
  ended_at: string;
}

export interface HistoryRun {
  id: number;
  session_id: number;
  status: string;
  trace_path: string;
  line_count: number;
  started_at: string;
  ended_at: string;
}

export interface HistoryExperiment {
  id: number;
  case_name: string;
  title: string;
  status: string;
  created_at: string;
  report_bytes: number;
}

export interface HistoryBruteJob {
  id: number;
  case_name: string;
  engine: string;
  status: string;
  hit_value: string;
  candidates_total: number;
  self_test_passed: boolean;
  perf_note: string;
}
