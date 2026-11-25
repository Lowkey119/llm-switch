import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

/** Typed IPC wrapper; every command mirrors src-tauri/src/commands/backend.rs */
export async function ipc<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  return invoke<T>(cmd, args);
}

/**
 * Tauri 事件订阅（后端 → 前端推送的唯一入路，与 ipc 同为出口封装）：
 * 返回 unlisten，调用方负责在卸载时解除。
 */
export async function ipcEvent<T>(
  event: string,
  handler: (payload: T) => void,
): Promise<() => void> {
  return listen<T>(event, (e) => handler(e.payload));
}

export interface BackendRow {
  id: number;
  name: string;
  base_url: string;
  protocol: string;
  api_key: string | null;
  weight: number;
  enabled: boolean;
  /** 直连模式：已接管的代理直连本后端真实地址，流量不经网关（三态 = 停用/网关/直连）。 */
  direct: boolean;
  /** 直连 Key 环境变量名（后端派生下发，前端只回显不拼串）。 */
  key_var: string;
  engine: string | null;
  /** 互斥配对引擎名（空 = 无配对）。当前生效引擎就是 engine 本身，切换见 halctl_use_engine。 */
  alt_engine: string | null;
  webui_url: string | null;
  webui_user: string | null;
  /** 服务端只回掩码；保存时原样送回 = 保持不变。 */
  webui_password: string | null;
  /** 本机启停脚本路径（空 = 该后端不提供启停；见 services::script）。 */
  start_script: string | null;
  stop_script: string | null;
  health_state: string;
  last_error: string | null;
  models: string[];
  aliases: Record<string, string>;
  /** 模型级开关（M12-A）：模型名可能含 / @ ?，所以只能随对象回来，不走 URL。 */
  model_flags?: ModelFlag[];
}

export interface ModelFlag {
  public_name: string;
  upstream_name: string;
  exposed: boolean;
  accept_upstream: boolean;
  /** 工种人工声明（11a）：chat / image_gen / tts / asr / video_gen；null = 未声明（回落判定链）。 */
  kind?: string | null;
}

export interface BackendInput {
  name: string;
  base_url: string;
  /** 编辑态带上被编辑那一行的原名。不带 = 新增，撞名即报错
   *  （后端按名字整行 UPDATE，不带原名的新增会把同名后端静默覆盖掉）。 */
  original_name?: string;
  protocol: string;
  api_key?: string | null;
  weight: number;
  enabled: boolean;
  engine?: string | null;
  models: string[];
  /** 「扫描模型」带回的真值：保存时一并落探测列（只落值，不动清单）。 */
  probe?: ScannedModel[];
  aliases?: Record<string, string>;
  webui_url?: string | null;
  webui_user?: string | null;
  webui_password?: string | null;
}

/** 扫描得到的单个模型真值：字段缺席 = 后端没给（未知不猜）。 */
export interface ScannedModel {
  id: string;
  ctx?: number | null;
  output?: number | null;
}

export interface ProbeOutcome {
  name: string;
  healthy: boolean;
  status: number | null;
  latency_ms: number;
  error: string | null;
}

/** 删后端时连带清掉的配置条数（服务端不默默删，UI 要如实转述）。 */
export interface DeleteOutcome {
  dropped_params: number;
  dropped_meta: number;
}

/** 控制台页「启动 / 停止」按钮的战报（见 services::script）。 */
export interface ScriptOutcome {
  /** true = 如预期完成（启动：仍在跑或脚本自报成功；停止：退出码 0）。 */
  ok: boolean;
  pid: number | null;
  exit_code: number | null;
  output: string;
  /** 启动脚本的完整输出落点（停止动作不落文件）。 */
  log: string | null;
}

export interface ShellStatus {
  version: string;
  listen: string;
  uptime_ms: number;
  backends: number;
  today_requests: number;
  today_tokens: number;
  /** 菜单栏快速启停：true = 已暂停接单（推理入口返回 503）。 */
  paused: boolean;
  /** 网关访问令牌（未设 = null）。控制台 iframe 发不出自定义请求头，
   *  只能把令牌拼进 /console/:id?access_token=… 的 URL（见 gateway/auth.rs）。 */
  access_token?: string | null;
}

/** 模型矩阵（M12-G）：一格 = 这个模型给不给这个端用。 */
export interface MatrixCell {
  in_list: boolean;
  is_default: boolean;
  /** 占了哪个槽位（第一个）：档位制 = 档位键（haiku/sonnet/…），pi = main/small。 */
  slot: string | null;
  /** 占的全部槽位：槽位之间不再互斥，同一个模型可以同时占好几个。 */
  slots?: string[];
}

export interface MatrixClient {
  app: string;
  label: string;
  taken_over: boolean;
  tiers: string[];
  /** 槽位键全集（后端下发）：档位制 = ["main", ...tiers]，pi = ["main","small"]，清单制 = []。 */
  slots: string[];
  capacity: number | null;
  /** 已占模型位（档位制 = 占槽 + 默认；gemini = 默认；清单制 = 清单长度）。 */
  used: number;
  /** false = 该端默认档自理（pi、copilot 门面），UI 不摆 ★。 */
  default_allowed: boolean;
  /** 网关门面（copilot，无配置文件可接管）：代理管理不列它，模型矩阵是唯一入口。 */
  facade: boolean;
  note: string | null;
  visible: string[];
  default: string | null;
}

export interface MatrixView {
  clients: MatrixClient[];
  rows: Array<{ model: string; backends: string[]; cells: Record<string, MatrixCell> }>;
}

/** 客户端列清单（两张矩阵同源 D24）：`client_list` 下发，两张矩阵的列都取自这里。
 * `enabled` = 代理启停态（代理 tab 片三）：停用的端由消费方滤掉列。 */
export interface ClientCol {
  app: string;
  label: string;
  enabled: boolean;
}

/** 一份配置文件落点（存在性来自接管评测的 files）。 */
export interface ConfigPath {
  path: string;
  exists: boolean;
}

/** 一个代理的安装扫描结果（commands/agents.rs::agents_scan）。 */
export interface AgentScan {
  app: string;
  label: string;
  /** 可执行命中 或 配置存在 = 视为已安装。 */
  installed: boolean;
  /** 可执行文件是否命中（区分「装了 CLI」与「只有配置残留」）。 */
  bin_found: boolean;
  /** 命中可执行但跑不起来。 */
  broken: boolean;
  /** 可执行探测是否已有结论；false = 冷启动首轮在飞，「没装」还不可信。 */
  probed: boolean;
  bin: string;
  bin_path: string | null;
  version: string | null;
  /** 一键安装命令（固定白名单）；null = 只给手动引导。 */
  install_command: string | null;
  config_paths: ConfigPath[];
}

/** `agents_scan` 的返回：快照 + 后台刷新标志。 */
export interface AgentScanResult {
  scans: AgentScan[];
  /** 后台探测在飞（冷启动 / 重扫）；扫完会 emit `agents://scan`，前端监听重取。 */
  scanning: boolean;
}

/** ②区模型指标（commands/panel.rs::model_metrics）：null = 无数据，UI 显 `—`。 */
export interface ModelMetric {
  model: string;
  streaming: boolean;
  prefill_tps: number | null;
  decode_tps: number | null;
  ctx_used: number | null;
  ctx_max: number | null;
  other_active: boolean;
  /** 图文能力：sup 支持 / uns 不支持 / unk 未知（默认放行）。 */
  vlm: "sup" | "uns" | "unk";
}

/** 模型元数据声明（commands/backend.rs::model_meta_get）：ctx/output 0 = 自动，>0 = 人工强制；reasoning/vlm 三态 0 自动/1 开/2 关。 */
export interface ModelMeta {
  model: string;
  /** 有效值（人工值已按后端上限钳过）；null = 未知不写。 */
  ctx: number | null;
  output: number | null;
  ctx_source: "manual" | "backend" | "unknown";
  output_source: "manual" | "backend" | "unknown";
  /** 思考能力 / 视觉（VLM）有效值：null = 未知不写。 */
  reasoning: boolean | null;
  vlm: boolean | null;
  reasoning_source: "manual" | "backend" | "unknown";
  vlm_source: "manual" | "backend" | "unknown";
  /** 人工声明原始值（0 = 自动）。 */
  manual_ctx: number;
  manual_output: number;
  manual_reasoning: number;
  manual_vlm: number;
  /** 后端允许范围上限（null = 无从得知，不拦）。 */
  cap_ctx: number | null;
  cap_output: number | null;
  /** 手工单价（USD/百万 token，片4）：null = 未设 = 在线也不计价；本地后端不计费。 */
  price: { input: number; output: number; cache_read: number; cache_creation: number } | null;
}

/** model_meta_set 返回：实际落库值 + 钳界提示语 + 受管端自动重写报告。 */
export interface ModelMetaSetResult {
  ctx: number;
  output: number;
  reasoning: number;
  vlm: number;
  cap_ctx: number | null;
  cap_output: number | null;
  clamped_ctx: boolean;
  clamped_output: boolean;
  message: string;
  /** 元数据变更后被自动重写的接管端列表（无变化不在列）。 */
  resynced?: unknown[];
}

// ── halogen 控制台（SSH 控制面）────────────────────────────────────────
/** halctl_status：容器状态 + health + cache + metrics 快照。 */
export interface HalStatus {
  reachable: boolean;
  host: string;
  running: boolean;
  container: string;
  health: Record<string, unknown> | null;
  cache: Record<string, unknown> | null;
  metrics: Record<string, number> | null;
}

/** halctl_versions：当前 tag + 本地镜像 + 备份槽 + 官方 tag 列表。 */
export interface HalVersions {
  current: string;
  local: string[];
  backup: { ver: string; full: string; when: string } | null;
  remote: string[];
  remote_error: string | null;
  host: string;
}

/** halctl_logs：中文渲染后的日志行。 */
export interface HalLogs {
  lines: string[];
  host: string;
  /** 远端读日志失败的原因（引擎停止时容器被删 → 「容器不存在」）；成功时为空。 */
  error?: string | null;
}

/** 控制/切换类命令的统一返回。 */
export interface HalOp {
  ok: boolean;
  output: string;
  host?: string;
}

// ── halogen 参数与环境变量（halogen.env + 命名预设）─────────────────
/** halctl_env：规格（分组白名单，真相在 102 的 hal_env.py）+ 当前值 + 预设。 */
export interface HalEnvSpecItem {
  key: string;
  label: string;
  kind: "num" | "txt" | "opt" | "key";
  opts: string[] | null;
}
export interface HalEnvGroup {
  group: string;
  items: HalEnvSpecItem[];
}
export interface HalEnvPreset {
  name: string;
  when: string;
  n: number;
}
export interface HalEnv {
  spec: HalEnvGroup[];
  values: Record<string, string>;
  presets: HalEnvPreset[];
  host: string;
}
