/**
 * llm-switch-rate — OpenCode / DevEco Code 的 TUI 速率插件（通用版：网关优先，直连也能看）
 * --------------------------------------------------------------------------
 * 两级取数，同一行展示：
 *
 *   1. **走 llm-switch**（优先）：数值一律取网关出口
 *      `GET {base}/admin/rates/line?app=<opencode|deveco>`（`gateway/rates.rs::line_json`），
 *      与浮窗单行速率线同源同口径——decode = 引擎原生速率优先，原生没报或恒 0 时退回
 *      网关校准值（响应字节 × 纠偏系数，显示带 ~）；prefill 同一套；上下文估算
 *      超窗不可信时网关给 null（画 —，绝不把窗口当已用，BUG-8 三修）。
 *      **本插件在这种模式下一个字都不算速率**，只取数与排版。
 *
 *   2. **直连在线 API / 网关没跑 / 网关查不到本代理**：退回**本地估算**。插件就住在
 *      代理进程里，各内核用各自已验证的路子拿「端点入口流量」：
 *      · OpenCode V2：`ctx.data` 轮询当前会话（`session.list/get` 的累计 tokens 每秒差分
 *        就是真实 output tok/s；`time.busy` 到首字的时间差量 prefill；input+cache 即 ctx）。
 *        实测 V2 的 `data.on` 事件流在插件上下文里不发货，轮询是唯一可靠路径。
 *      · DevEco V1：`api.event.on` 订阅自己的消息流（`message.updated` /
 *        `message.part.updated`），3s 滑动窗口的文本增量 × 逐模型 tok 系数（系数用
 *        step-finish 的真实 output tokens 在线校准）。
 *      本地口径没有引擎字节，精度天然不如网关，全部带 `~` 如实标注，不冒充网关口径。
 *
 * 两条路径共用同一个槽、同一份 signal：每秒先问网关，网关有行就全文照抄，
 * 没有行再落本地估算——所以走 llm-switch 时行为与纯网关版完全一致。
 *
 * ── 两家内核的 TUI 插件契约不是一个版本，必须同时满足 ──────────────────────────
 * · OpenCode（V2，≥1.18）：`setup(ctx)`，`ctx.ui.slot({ append: "<槽名>", render })`
 *   逐槽认领（槽名点号风格：`home.footer.status` / `prompt.footer.status`），
 *   返回清理函数。少了 `setup` 整插件不加载。
 * · DevEco Code（V1 内核）：`tui(api)`，`api.slots.register({ order, slots })`
 *   一次登记（槽名下划线风格：`home_footer` / `session_prompt_right`）。
 * 两个入口都留着、各自探测：谁调用谁生效，另一个原样返回不管。
 *
 * 装载（由 llm-switch 的「扩展 · 一键安装」写盘，两端同一份源码）：
 *   OpenCode   ~/.config/opencode/plugins/llm-switch-rate/tui.ts  （V2 自动发现，无需登记）
 *   DevEco     ~/.config/deveco/plugin/llm-switch-rate.ts         （deveco.jsonc 的 plugin 数组登记）
 *
 * 端点发现（顺序取第一个命中的）：
 *   1. `LLMSWITCH_RATE_URL` + `LLMSWITCH_RATE_TOKEN`
 *   2. `<LLM_SWITCH_DATA_DIR|~/.llm-switch>/rate-ep.json`
 *
 * 诊断：进程启动会在 /tmp 留 `llm-switch-rate.loaded`（模块+轮询已起，含宿主 pid/cwd）
 * 与 `llm-switch-rate.setup-v2` / `.setup-v1`（对应内核入口已跑）。文件缺失 = 没加载上。
 */

import { readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { createSignal } from "solid-js";
import { jsx as h } from "@opentui/solid/jsx-runtime";

const POLL_MS = 1000;
const TIMEOUT_MS = 1500;
const PHASE_RANK: Record<string, number> = { generating: 0, prefill: 1, queued: 2 };
/** 本地估算（V1 事件路）的窗口与节奏（旧 token-rate 真机调出来的常数）。 */
const WINDOW_MS = 3000;
const IDLE_MS = 1500;
const INIT_TPC = 0.3;
const CAL_ALPHA = 0.35;
const MIN_CAL_CHARS = 120;
const MAX_TPC = 2;

/** OpenCode V2 槽名（点号风格）：首页页脚状态区 + 会话输入框状态区。 */
const V2_SLOTS = ["home.footer.status", "prompt.footer.status"];
/** DevEco V1 槽名（下划线风格）：首页页脚 + 会话输入框右侧。 */
const V1_SLOTS = ["home_footer", "session_prompt_right"];
/** V1 消息流事件名（DevEco 内核实测）。 */
const V1_EVENTS = ["message.updated", "message.part.updated"];

/** 代理名：插件被两家内核共用，按安装路径的宿主标识分（env 可覆盖）。 */
function appName(): string {
  const env = process.env.LLMSWITCH_RATE_APP;
  if (env) return env;
  try {
    const url = String(import.meta.url);
    if (url.includes("deveco")) return "deveco";
    if (url.includes("opencode")) return "opencode";
  } catch {
    // import.meta.url 拿不到（个别内核把模块打包没了 URL）→ 退回环境变量。
  }
  return String(process.env.DEVECO_HOME || process.env.OPENCODE_HOME || "").includes("deveco")
    ? "deveco"
    : "opencode";
}

/** 诊断面包屑：每个进程启动只写一两次小文件到 /tmp，排查「装了怎么没显示」用。 */
function crumb(name: string, data: unknown): void {
  try {
    writeFileSync(`/tmp/llm-switch-rate.${name}`, JSON.stringify(data));
  } catch {
    // 面包屑写不了就算了，绝不影响速率显示。
  }
}

function endpoint(): { base: string; token: string } | null {
  const envBase = (process.env.LLMSWITCH_RATE_URL || "").replace(/\/+$/, "");
  if (envBase) return { base: envBase, token: process.env.LLMSWITCH_RATE_TOKEN || "" };
  const dir = process.env.LLM_SWITCH_DATA_DIR || join(homedir(), ".llm-switch");
  try {
    const j = JSON.parse(readFileSync(join(dir, "rate-ep.json"), "utf8"));
    const base = String(j.base || "").replace(/\/+$/, "");
    return base ? { base, token: j.token || "" } : null;
  } catch {
    return null;
  }
}

async function fetchLine(app: string): Promise<any | null> {
  const ep = endpoint();
  if (!ep) return null;
  const ctl = new AbortController();
  const timer = setTimeout(() => ctl.abort(), TIMEOUT_MS);
  try {
    // pids=本进程：多实例分流（BUG-1）——本扩展跑在代理进程内，pid 就是它自己，
    // 网关据此只看这个实例发的请求，同家开第二个窗口互不串数。
    const res = await fetch(`${ep.base}/admin/rates/line?app=${encodeURIComponent(app)}&pids=${process.pid}`, {
      headers: ep.token ? { Authorization: `Bearer ${ep.token}` } : {},
      signal: ctl.signal,
    });
    if (!res.ok) return null;
    const v = await res.json();
    return v && v.ok ? v : null;
  } catch {
    return null;
  } finally {
    clearTimeout(timer);
  }
}

/** tok/s 展示：≥100 取整，否则一位小数；没数 → —。 */
function tps(n: any): string {
  if (n == null || !Number.isFinite(n)) return "—";
  return n >= 100 ? String(Math.round(n)) : n.toFixed(1);
}

// ── 网关口径：一行文案（与浮窗单行速率线一致）───────────────────────────────

function text(rows: any[]): string {
  if (!rows.length) return "";
  const r = [...rows].sort(
    (a, b) => (PHASE_RANK[a.phase] ?? 9) - (PHASE_RANK[b.phase] ?? 9),
  )[0];
  // 槽位只放得下一行：多于一时在飞时挂 `#N`（网关按挂号先后编号），告诉你「这是第 N 条」。
  // 网关分不出主/子代理（同进程同入口同模型），序号只分开并发行，不表示主次。
  const seq = rows.length > 1 ? `#${r.seq ?? 1} ` : "";
  // 只显示 decode：prefill 与上下文长度各代理自己显示，这里不重复占位。
  // prefill 在飞（还没出第一个字）没有 decode 可说 → 留空，等 decode 到了再亮。
  if (r.phase === "queued" || r.phase === "prefill") return "";
  // 网关出口就是计算值（原生或字节校准），全文照抄不加 ~：~ 只留给本地估算。
  return `${seq}▶ ${tps(r.decode_tps)} tok/s`;
}

// ── 本地估算 A（DevEco V1）：订阅自己的消息流 ───────────────────────────────

/** 估计器状态。旧 token-rate.ts 的路子（DevEco 真机验证过）：事件只记账
 *  （增量、首字时刻、step-finish 的真实 tokens），rate 一律在 tick 里现算。 */
const est = {
  samples: [] as { t: number; n: number }[],
  partLens: new Map<string, number>(),
  stepStarts: new Map<string, number>(),
  firstTokenAt: new Map<string, number>(),
  lastDeltaAt: new Map<string, number>(),
  stepChars: new Map<string, number>(),
  /** 逐模型 tok/char 系数：step-finish 拿真实 output tokens 校准（指数滑动）。 */
  coefs: new Map<string, number>(),
  model: "",
  /** 最近一次 step-finish 量出来的均值与上下文（空闲展示用）。 */
  lastPrefill: 0,
  lastAvgOut: 0,
  usedCtx: 0,
  ctxWindow: 0,
  /** 在飞 prefill：assistant 消息开了但还没等到第一个字。 */
  pendingId: "",
  pendingStart: 0,
  /** 诊断：收到过第一条事件 / 第一次算出非空本地文案时各落一次面包屑。 */
  firstEventSeen: false,
  firstLocalSeen: false,
};

function onMessageUpdated(e: any): void {
  const info = e?.properties?.info ?? e?.info ?? {};
  if (info.role === "assistant") {
    if (typeof info.modelID === "string") est.model = info.modelID;
    const mid = info.id as string | undefined;
    const created = (info.time?.created as number | undefined) ?? Date.now();
    if (mid && created && !est.stepStarts.has(mid)) {
      est.stepStarts.set(mid, created);
      est.pendingId = mid;
      est.pendingStart = created;
    }
  } else if (info.role === "user") {
    // 新一轮开始：上一轮的窗口记账清零（均值保留，空闲展示还要用）。
    est.samples = [];
    est.pendingId = "";
    est.firstTokenAt.clear();
    est.lastDeltaAt.clear();
    est.stepStarts.clear();
    est.stepChars.clear();
  }
}

function onPartUpdated(e: any): void {
  const part = e?.properties?.part ?? e?.part ?? {};
  if (part.type === "step-start") {
    if (!est.stepStarts.has(part.messageID)) est.stepStarts.set(part.messageID, Date.now());
  } else if (part.type === "text" || part.type === "reasoning") {
    const len = typeof part.text === "string" ? part.text.length : 0;
    const prev = est.partLens.get(part.id) ?? 0;
    est.partLens.set(part.id, len);
    const delta = len - prev;
    if (delta <= 0) return;
    const now = Date.now();
    est.samples.push({ t: now, n: delta });
    est.stepChars.set(part.messageID, (est.stepChars.get(part.messageID) ?? 0) + delta);
    est.lastDeltaAt.set(part.messageID, now);
    if (!est.firstTokenAt.has(part.messageID)) {
      est.firstTokenAt.set(part.messageID, now);
      if (part.messageID === est.pendingId) est.pendingId = "";
    }
  } else if (part.type === "tool") {
    // tool 调用参数也是流式到达的 token：漏掉它们，工具密集的回合会把速率压低一个量级。
    let len = 0;
    try {
      len = JSON.stringify(part?.state?.input ?? "").length;
    } catch {
      len = 0;
    }
    const prev = est.partLens.get(part.id) ?? 0;
    est.partLens.set(part.id, len);
    const delta = len - prev;
    if (delta <= 0) return;
    const now = Date.now();
    est.samples.push({ t: now, n: delta });
    est.stepChars.set(part.messageID, (est.stepChars.get(part.messageID) ?? 0) + delta);
    est.lastDeltaAt.set(part.messageID, now);
    if (!est.firstTokenAt.has(part.messageID)) est.firstTokenAt.set(part.messageID, now);
  } else if (part.type === "step-finish") {
    if (part.messageID === est.pendingId) est.pendingId = "";
    const start = est.stepStarts.get(part.messageID) ?? 0;
    const first = est.firstTokenAt.get(part.messageID) ?? 0;
    const done = est.lastDeltaAt.get(part.messageID) ?? first;
    const cache = part.tokens?.cache ?? {};
    const contextTokens = (part.tokens?.input ?? 0) + (cache.read ?? 0) + (cache.write ?? 0);
    if (contextTokens) {
      est.usedCtx = contextTokens;
      est.ctxWindow = part.tokens?.contextWindow ?? est.ctxWindow;
    }
    if (start > 0 && first > start && contextTokens > 0) {
      est.lastPrefill = contextTokens / ((first - start) / 1000);
    }
    const chars = est.stepChars.get(part.messageID) ?? 0;
    const gen = (part.tokens?.output ?? 0) + (part.tokens?.reasoning ?? 0);
    if (first > start && done > first && gen > 0) {
      est.lastAvgOut = gen / ((done - first) / 1000);
    }
    // 逐模型系数校准：真实 tokens ÷ 数到的字符数，往旧系数上滑一点（防抖）。
    if (chars >= MIN_CAL_CHARS && gen > 0 && est.model !== "") {
      const ratio = gen / chars;
      if (ratio > 0 && ratio <= MAX_TPC) {
        const old = est.coefs.get(est.model) ?? INIT_TPC;
        est.coefs.set(est.model, old * (1 - CAL_ALPHA) + ratio * CAL_ALPHA);
      }
    }
  }
}

/** V1 本地口径的一行文案（带 ~ 标注「估计」）。只显示 decode：
 *  prefill 在飞（还没出第一个字）没有 decode 可说 → ""，历史由外层统一加「history:」。 */
function estText(now: number): string {
  est.samples = est.samples.filter((s) => now - s.t <= WINDOW_MS);
  const last = est.samples.length > 0 ? est.samples[est.samples.length - 1].t : 0;
  if (now - last > IDLE_MS) return "";
  const chars = est.samples.reduce((a, s) => a + s.n, 0);
  const span = Math.max(250, Math.min(WINDOW_MS, now - est.samples[0].t));
  const tpc = est.coefs.get(est.model) ?? INIT_TPC;
  return `▶ ~${tps((chars * tpc) / (span / 1000))} tok/s`;
}

/** V1 事件记账挂到 `api.event`，返回退订列表。 */
function attachV1Events(api: any): (() => void)[] {
  const unlisten: (() => void)[] = [];
  const route: Record<string, (e: any) => void> = {
    "message.updated": onMessageUpdated,
    "message.part.updated": onPartUpdated,
  };
  for (const name of V1_EVENTS) {
    try {
      const off = api.event.on(name, (e: any) => {
        if (!est.firstEventSeen) {
          est.firstEventSeen = true;
          crumb("first-event", { name, keys: e ? Object.keys(e) : null });
        }
        try {
          route[name](e);
        } catch {
          // 记账出错不许影响显示：跳过这一个事件。
        }
      });
      if (typeof off === "function") unlisten.push(off);
    } catch {
      // 这个事件名在该内核不存在：少一路记账而已，估算继续。
    }
  }
  return unlisten;
}

// ── 本地估算 B（OpenCode V2）：轮询消息流的内容增量（每块口径）───────────────

/** V2 实测：`data.on` 事件流在插件上下文里不发货；`data.session.*` 轮询可靠。
 *  会话级 tokens 只在 step 结束才提交、流式期间不动——所以 decode 只能按「块」估：
 *  每次轮询量到的内容长度增量就是这一拍到达的块，滑动窗口 × 逐模型 tok 系数
 *  （系数用 step 结束时的真实 tokens 在线校准）。与网关「响应字节 × 纠偏系数」同构，
 *  只是这里量的是字符不是字节，一律带 ~ 如实标注，不冒充网关口径。 */
const v2 = {
  /** 滑动窗口样本（t = 时刻，n = 这一拍到的字符增量）。 */
  samples: [] as { t: number; n: number }[],
  /** 最后一条 assistant 消息的 id 与内容长度（跨消息时重置差分）。 */
  msgId: "",
  msgLen: 0,
  /** 逐模型 tok/char 系数：step 结束时用真实 tokens 校准（指数滑动）。 */
  coefs: new Map<string, number>(),
  model: "",
  /** 本轮是否已等到第一个字（量一次 prefill 用）。 */
  firstAt: 0,
  /** 会话转 busy 的时刻（prefill 起点）。 */
  busyAt: 0,
  prefillTps: 0,
  usedCtx: 0,
  ctxWindow: 0,
};

/** assistant 消息已到达的正文长度（text + reasoning + 在飞的 tool 入参的字符数）。
 *  tool 调用参数也是流式到达的 token：不算它们，工具密集的回合会把速率压低一个量级。 */
function msgTextLen(m: any): number {
  const parts: any[] = Array.isArray(m?.content) ? m.content : [];
  let n = 0;
  for (const p of parts) {
    if ((p?.type === "text" || p?.type === "reasoning") && typeof p.text === "string") {
      n += p.text.length;
    } else if (p?.type === "tool") {
      try {
        n += JSON.stringify(p?.state?.input ?? "").length;
      } catch {
        /* 序列化不了就不计 */
      }
    }
  }
  return n;
}

async function pollLocal(ctxData: any, sessionID: string | null): Promise<string> {
  // 找会话：TUI 正在看的优先；列表里最近更新的兜底（V2.2.7 实测 get/list 都通）。
  let s: any = null;
  if (sessionID) {
    try {
      s = await ctxData.session?.get?.(sessionID);
    } catch {
      s = null;
    }
  }
  if (!s) {
    try {
      const list = await ctxData.session?.list?.();
      if (Array.isArray(list) && list.length) {
        s = list.reduce((a: any, b: any) =>
          (b?.time?.updated ?? 0) > (a?.time?.updated ?? 0) ? b : a,
        );
      }
    } catch {
      s = null;
    }
  }
  if (!s) return "";
  const now = Date.now();
  // 在飞判定：status 端点优先（实测返回 "running"），time.busy/idle 兜底。
  let running = false;
  try {
    const st = await ctxData.session?.status?.(s.id ?? sessionID);
    running = String(st).includes("running");
  } catch {
    running = (s.time?.busy ?? 0) > (s.time?.idle ?? 0);
  }
  if (!running) {
    v2.samples = [];
    v2.msgId = "";
    v2.msgLen = 0;
    v2.firstAt = 0;
    v2.busyAt = 0;
    return "";
  }
  if (typeof s.model?.modelID === "string") v2.model = s.model.modelID;
  else if (typeof s.model?.id === "string") v2.model = s.model.id;
  // 新一轮 busy：窗口记账清零，prefill 起点取会话转 busy 的时刻。
  const busy = s.time?.busy ?? 0;
  if (busy && busy !== v2.busyAt) {
    v2.busyAt = busy;
    v2.firstAt = 0;
    v2.samples = [];
    v2.msgId = "";
    v2.msgLen = 0;
  }
  // 拉消息：最后一条 assistant 消息的正文长度就是「已到达的块」。
  let msgs: any[] = [];
  try {
    const r = await ctxData.session?.message?.list?.(s.id);
    msgs = Array.isArray(r) ? r : [];
  } catch {
    msgs = [];
  }
  let last: any = null;
  for (let i = msgs.length - 1; i >= 0; i--) {
    const m = msgs[i];
    if (m?.type === "assistant" || m?.role === "assistant") {
      last = m;
      break;
    }
  }
  if (!last) return "";
  const mid = String(last.id ?? "");
  const len = msgTextLen(last);
  if (mid !== v2.msgId) {
    // 新消息（或插件中途挂上）：首拍只做基线，别把已有内容当成一次性增量算出虚高。
    v2.msgId = mid;
    v2.msgLen = len;
    v2.samples = [];
  }
  const delta = len - v2.msgLen;
  if (delta > 0) {
    v2.samples.push({ t: now, n: delta });
    if (!v2.firstAt) v2.firstAt = now;
  }
  v2.msgLen = len;
  // 真实 tokens（step 结束才提交）：校准逐模型系数 + 取真实 ctx。
  const tk = last.tokens ?? {};
  const c = tk.cache ?? {};
  const ctxUsed = (tk.input ?? 0) + (c.read ?? 0) + (c.write ?? 0);
  if (ctxUsed) {
    v2.usedCtx = ctxUsed;
    v2.ctxWindow = tk.contextWindow ?? v2.ctxWindow;
  }
  const gen = (tk.output ?? 0) + (tk.reasoning ?? 0);
  if (gen > 0 && len >= MIN_CAL_CHARS && v2.model !== "") {
    const ratio = gen / len;
    if (ratio > 0 && ratio <= MAX_TPC) {
      const old = v2.coefs.get(v2.model) ?? INIT_TPC;
      v2.coefs.set(v2.model, old * (1 - CAL_ALPHA) + ratio * CAL_ALPHA);
    }
  }
  v2.samples = v2.samples.filter((x) => now - x.t <= WINDOW_MS);
  if (!v2.samples.length) return "";
  const chars = v2.samples.reduce((a, x) => a + x.n, 0);
  const span = Math.max(250, Math.min(WINDOW_MS, now - v2.samples[0].t));
  const tpc = v2.coefs.get(v2.model) ?? INIT_TPC;
  return `▶ ~${tps((chars * tpc) / (span / 1000))} tok/s`;
}

// ── 装配：一份轮询 + 一条 signal，两个内核各认各的槽 ────────────────────────

/** 本地文案提供器：V1 给同步的 estText，V2 给异步的 pollLocal。 */
type LocalFn = () => string | Promise<string>;

interface Live {
  render: () => any;
  stop: () => void;
  unlisten: (() => void)[];
}

function start(local: LocalFn): { render: () => any; stop: () => void } {
  const app = appName();
  const [label, setLabel] = createSignal("");
  let stopped = false;
  /** 最近一次在飞时显示的速率行（网关口径或本地口径都算）。 */
  let lastText = "";
  /** 诊断：第一次算出非空本地文案 / 第一次本地估算报错，各落一次面包屑。 */
  let localSeen = false;
  let errSeen = false;

  const tick = async () => {
    if (stopped) return;
    // 网关优先：有行照抄（llm-switch 口径，一个字不算）；没有行落本地估算。
    const v = await fetchLine(app);
    if (stopped) return;
    const rows = (v?.rows as any[] | undefined) ?? [];
    if (rows.length) {
      lastText = text(rows);
      if (lastText) {
        setLabel(lastText);
        return;
      }
    }
    let t = "";
    try {
      t = (await local()) ?? "";
    } catch (e) {
      if (!errSeen) {
        errSeen = true;
        crumb("local-error", String(e).slice(0, 200));
      }
      t = "";
    }
    if (stopped) return;
    if (t && !localSeen) {
      localSeen = true;
      crumb("local-text", { app, t: t.slice(0, 120) });
    }
    if (t) {
      lastText = t;
      setLabel(t);
    } else if (lastText) {
      // 流停了：最后一行速率前加「history:」前缀（不灰显、不加后缀）。
      setLabel(`history: ${lastText}`);
    } else {
      setLabel("");
    }
  };
  const timer = setInterval(tick, POLL_MS);
  void tick();

  const render = () => h("text", { fg: "#7d8590", children: () => label() });
  const stop = () => {
    stopped = true;
    clearInterval(timer);
  };
  crumb("loaded", {
    app,
    at: Date.now(),
    pid: process.pid,
    cwd: (() => {
      try {
        return process.cwd();
      } catch {
        return null;
      }
    })(),
  });
  return { render, stop };
}

/** 同一进程里 setup / tui 只可能有一个真正生效；万一都被调，第二次复用第一份。 */
let live: Live | null = null;

function mountShared(local: LocalFn): Live {
  if (live) return live;
  const s = start(local);
  live = { ...s, unlisten: [] };
  return live;
}

export default {
  id: "llm-switch-rate",

  /** OpenCode V2：`ctx.ui.slot` 逐槽认领 + `ctx.data` 轮询本地差分，返回清理函数。 */
  setup: async (ctx: any) => {
    // setup 也会被非 TUI 的上下文探一次（服务端形态没有 slot）——没有 slot 就什么都不做。
    if (!ctx || typeof ctx.ui?.slot !== "function") return;
    // 当前正在看的会话（在会话路由里才有）：本地估算的取数对象。
    const routeSession = (): string | null => {
      try {
        const cur = typeof ctx.ui?.router?.current === "function" ? ctx.ui.router.current() : null;
        if (cur?.type === "session" && cur.sessionID) return String(cur.sessionID);
      } catch {
        // 拿不到路由就交给 session.list 兜底。
      }
      return null;
    };
    const { render, stop } = mountShared(() => pollLocal(ctx.data, routeSession()));
    const attached: string[] = [];
    for (const name of V2_SLOTS) {
      try {
        ctx.ui.slot({ append: name, render });
        attached.push(name);
      } catch {
        // 某个槽在这个版本不存在（内核换代时真会发生）：跳过它，其余照挂。
      }
    }
    crumb("setup-v2", { attached });
    return () => {
      if (live && live.stop === stop) live = null;
      stop();
    };
  },

  /** DevEco Code（V1 内核）：`api.slots.register` + `api.event.on` 记账。 */
  tui: async (api: any) => {
    if (!api || typeof api.slots?.register !== "function") return;
    const { render, stop } = mountShared(() => estText(Date.now()));
    const slots: Record<string, () => any> = {};
    for (const name of V1_SLOTS) slots[name] = render;
    try {
      api.slots.register({ order: 100, slots });
      crumb("setup-v1", { slots: V1_SLOTS });
    } catch (e) {
      crumb("setup-v1-error", String(e));
    }
    if (typeof api.event?.on === "function") {
      live!.unlisten = attachV1Events(api);
    }
    return () => {
      for (const off of live!.unlisten.splice(0)) {
        try {
          off();
        } catch {
          // 退订失败就算了。
        }
      }
      if (live && live.stop === stop) live = null;
      stop();
    };
  },
};
