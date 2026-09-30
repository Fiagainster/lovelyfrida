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
  target: string | null;
  session_id: number | null;
  script_id: number | null;
  hello: Record<string, unknown> | null;
  channel: string;
  updated_at: string;
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
  fridaForwardSetup: () => invoke<StepReport>("frida_forward_setup"),
  fridaProcesses: () => invoke<ProcEntry[]>("frida_processes"),
  fridaSessionAttach: (target: number | string) =>
    invoke<SessionSnapshot>("frida_session_attach", { target }),
  fridaSessionDetach: () => invoke<SessionSnapshot>("frida_session_detach"),
  fridaSessionStatus: () => invoke<SessionSnapshot>("frida_session_status"),
  fridaSessionPing: () => invoke<Record<string, unknown>>("frida_session_ping"),

  confirmClose: () => invoke<void>("confirm_close"),

  // 终端（PTY）
  terminalCreate: (serial: string) => invoke<TerminalInfo>("terminal_create", { serial }),
  terminalWrite: (id: number, data: string) => invoke<void>("terminal_write", { id, data }),
  terminalResize: (id: number, cols: number, rows: number) =>
    invoke<void>("terminal_resize", { id, cols, rows }),
  terminalClose: (id: number) => invoke<void>("terminal_close", { id }),

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
