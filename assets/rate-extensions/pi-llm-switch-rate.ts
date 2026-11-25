/**
 * llm-switch-rate — 输入行上方的速率条（llm-switch 速率口径的瘦客户端 · 通用版）
 * --------------------------------------------------------------------------
 * 两级取数，同一行展示：
 *
 *   1. **走 llm-switch**（优先）：数值一律取网关出口
 *      `GET {base}/admin/rates/line?app=pi`（`gateway/rates.rs::line_json`），与浮窗
 *      单行速率线同源同口径——decode = 引擎原生速率优先，原生没报或恒 0 时退回网关
 *      校准值（响应字节 × 纠偏系数，显示带 ~）；prefill 同一套。归属由网关从 socket
 *      反查（`gateway/origin.rs`），这里只按 `app=pi` 过滤。这种模式一个字都不算。
 *
 *   2. **直连在线 API / 网关没跑 / 网关查不到本代理**：退回**本地估算**——扩展就住在
 *      pi 进程里，订阅它自己的消息流（`message_start` / `message_update` /
 *      `message_end`），逐块（text_delta / thinking_delta）记账：decode = 3s 滑动窗口
 *      的字符增量 × 逐模型 tok 系数（`message_end` 的 usage.output 在线校准，带 ~）；
 *      prefill = 上下文 ÷（首块 − 请求起点）。本地口径没有引擎字节，精度不如网关，
 *      全部带 ~ 如实标注。
 *
 * 只显示 prefill 与 decode：上下文长度 pi 自己显示，这里不重复占位。
 *
 * 端点发现（顺序取第一个命中的）：
 *   1. `LLMSWITCH_RATE_URL` + `LLMSWITCH_RATE_TOKEN`
 *   2. `<LLM_SWITCH_DATA_DIR|~/.llm-switch>/rate-ep.json`（llm-switch 启动/改端口时写）
 *
 * 用法：装进 `~/.pi/agent/extensions/`（pi 自动扫描），重启 pi 生效；
 *   /llm-switch-rate off  隐藏状态条
 *   /llm-switch-rate on   显示状态条
 */

import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";
import { readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

const APP = "pi";
const POLL_MS = 1000;
const TIMEOUT_MS = 1500;
/** 本地估算的窗口与节奏（与 TUI 版同一套常数，真机调出来的）。 */
const WINDOW_MS = 3000;
const IDLE_MS = 1500;
const INIT_TPC = 0.3;
const CAL_ALPHA = 0.35;
const MIN_CAL_CHARS = 120;
const MAX_TPC = 2;

/** 网关出口的一行（`line_json` 的 rows 元素）。 */
interface RateRow {
	phase: string;
	model: string | null;
	decode_tps: number | null;
	prefill_tps: number | null;
	/** 已用上下文（网关先验链后的终值）；null = 估算超窗不可信，画 —，
	 *  绝不把窗口当已用（BUG-8 三修）。本扩展只渲染 decode，类型跟契约走。 */
	used_ctx: number | null;
	ctx_window: number;
	/** 数值来源：native = 引擎原生，gateway = 校准估算（带 ~），none = 没数。 */
	source: string;
	/** 并发序号（网关按挂号先后给的 1 起）：同一家多条在飞时用来分行，不表示主次。 */
	seq: number;
}

interface LineView {
	ok: boolean;
	rows: RateRow[];
}

const SPINNER_FRAMES = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const PHASE_RANK: Record<string, number> = { generating: 0, prefill: 1, queued: 2 };

const state = {
	frame: 0,
	rows: [] as RateRow[],
	/** 上一次成功取数的时刻；取不到（网关没跑）时置 0，状态条如实说明。 */
	updatedAt: 0,
	timer: 0 as ReturnType<typeof setInterval> | 0,
	/** 本地估算的一行文案（网关没行时顶上；空串 = 没数可说）。 */
	local: "",
	/** 最近一次在飞时显示的速率行：流停了在它前面加「history:」前缀。 */
	lastText: "",
};

let enabled = true;

function endpoint(): { base: string; token: string } | null {
	const envBase = (process.env.LLMSWITCH_RATE_URL || "").replace(/\/+$/, "");
	if (envBase) return { base: envBase, token: process.env.LLMSWITCH_RATE_TOKEN || "" };
	const dir = process.env.LLM_SWITCH_DATA_DIR || join(homedir(), ".llm-switch");
	try {
		const j = JSON.parse(readFileSync(join(dir, "rate-ep.json"), "utf8")) as {
			base?: string;
			token?: string;
		};
		const base = (j.base || "").replace(/\/+$/, "");
		return base ? { base, token: j.token || "" } : null;
	} catch {
		return null;
	}
}

async function fetchLine(): Promise<LineView | null> {
	const ep = endpoint();
	if (!ep) return null;
	const ctl = new AbortController();
	const timer = setTimeout(() => ctl.abort(), TIMEOUT_MS);
	try {
		// pids=本进程：多实例分流（BUG-1）——本扩展跑在 pi 进程内，pid 就是它自己。
		const res = await fetch(`${ep.base}/admin/rates/line?app=${encodeURIComponent(APP)}&pids=${process.pid}`, {
			headers: ep.token ? { Authorization: `Bearer ${ep.token}` } : {},
			signal: ctl.signal,
		});
		if (!res.ok) return null;
		const v = (await res.json()) as LineView;
		return v && v.ok ? v : null;
	} catch {
		return null;
	} finally {
		clearTimeout(timer);
	}
}

/** tok/s 展示：≥100 取整，否则一位小数；没数 → —（与浮窗 rate() 同口径）。 */
function tps(n: number | null): string {
	if (n == null || !Number.isFinite(n)) return "—";
	return n >= 100 ? String(Math.round(n)) : n.toFixed(1);
}

// ── 本地估算：pi 自己的消息流就是「端点入口流量」─────────────────────────────

/** 估计器状态：事件只记账（逐块字符增量、首块时刻、usage 的真实 tokens），
 *  rate 一律在 tick 里现算。与 TUI 版 V1 估算同一套路子。 */
const est = {
	samples: [] as { t: number; n: number }[],
	/** 在飞 assistant 消息：请求起点、首块时刻、已数到的字符。 */
	startTs: 0,
	firstAt: 0,
	chars: 0,
	model: "",
	/** 逐模型 tok/char 系数：message_end 的 usage.output 校准（指数滑动）。 */
	coefs: new Map<string, number>(),
	/** 最近一次 message_end 量出来的均值（空闲展示用）。 */
	lastPrefill: 0,
	lastAvgOut: 0,
};

function onMessageStart(m: any): void {
	if (m?.role !== "assistant") return;
	est.startTs = typeof m.timestamp === "number" ? m.timestamp : Date.now();
	est.firstAt = 0;
	est.chars = 0;
	if (typeof m.model === "string") est.model = m.model;
}

function onMessageUpdate(ev: any): void {
	const t = ev?.type;
	// toolcall_delta 也是流式到达的 token：漏掉它，工具密集的回合会把速率压低一个量级。
	if (t !== "text_delta" && t !== "thinking_delta" && t !== "toolcall_delta") return;
	const n = typeof ev.delta === "string" ? ev.delta.length : 0;
	if (n <= 0) return;
	const now = Date.now();
	if (!est.firstAt) est.firstAt = now;
	est.samples.push({ t: now, n });
	est.chars += n;
}

function onMessageEnd(m: any): void {
	if (m?.role !== "assistant") return;
	const u = m.usage ?? {};
	const ctxTokens = (u.input ?? 0) + (u.cacheRead ?? 0) + (u.cacheWrite ?? 0);
	const gen = u.output ?? 0;
	const now = Date.now();
	if (est.firstAt && est.startTs && ctxTokens > 0) {
		est.lastPrefill = ctxTokens / Math.max(0.5, (est.firstAt - est.startTs) / 1000);
	}
	if (est.firstAt && gen > 0) {
		est.lastAvgOut = gen / Math.max(0.5, (now - est.firstAt) / 1000);
	}
	// 逐模型系数校准：真实 tokens ÷ 数到的字符数，往旧系数上滑一点（防抖）。
	if (est.chars >= MIN_CAL_CHARS && gen > 0 && est.model !== "") {
		const ratio = gen / est.chars;
		if (ratio > 0 && ratio <= MAX_TPC) {
			const old = est.coefs.get(est.model) ?? INIT_TPC;
			est.coefs.set(est.model, old * (1 - CAL_ALPHA) + ratio * CAL_ALPHA);
		}
	}
}

/** 本地口径的一行文案（带 ~ 标注「估计」）。只显示 decode：
 *  prefill 在飞没有 decode 可说 → ""，历史由 tick 统一加「history:」。 */
function localText(now: number): string {
	est.samples = est.samples.filter((s) => now - s.t <= WINDOW_MS);
	const last = est.samples.length > 0 ? est.samples[est.samples.length - 1].t : 0;
	if (now - last > IDLE_MS) return "";
	const chars = est.samples.reduce((a, s) => a + s.n, 0);
	const span = Math.max(250, Math.min(WINDOW_MS, now - est.samples[0].t));
	const tpc = est.coefs.get(est.model) ?? INIT_TPC;
	return `▶ ~${tps((chars * tpc) / (span / 1000))} tok/s`;
}

function rowText(theme: ExtensionContext["theme"], r: RateRow): string {
	const model = theme.fg("accent", r.model ?? "未知模型");
	// 只显示 decode：prefill 与上下文长度 pi 自己显示，这里不重复占位。
	// prefill 在飞（还没出第一个字）没有 decode 可说 → 空串，等 decode 到了再亮。
	if (r.phase === "queued" || r.phase === "prefill") return "";
	// 网关出口就是计算值（原生或字节校准），全文照抄不加 ~：~ 只留给本地估算。
	return `${model} ${theme.fg("success", `▶ ${tps(r.decode_tps)} tok/s`)}`;
}

function buildContent(theme: ExtensionContext["theme"]): string[] {
	state.frame = (state.frame + 1) % SPINNER_FRAMES.length;
	// 网关有行 → 全文照抄（llm-switch 口径）；没行 → 本地估算顶上；
	// 流停了 → 最后一行速率前加「history:」前缀；都没有 → 收起。
	// 空 widget 不挂「llm-switch 未运行」：它每秒重画，常驻告警只会变成输入行上方的噪音。
	const rows = [...state.rows].sort((a, b) => (PHASE_RANK[a.phase] ?? 9) - (PHASE_RANK[b.phase] ?? 9));
	const pairs = rows.map((r) => ({ r, t: rowText(theme, r) })).filter((p) => p.t !== "");
	if (pairs.length > 0) {
		// 只有一条时不挂序号（`#1` 是纯噪音）；多于一行才编号，好让并发的行彼此分得开。
		const many = pairs.length > 1;
		return pairs.map(
			({ r, t }) =>
				`${theme.fg("accent", SPINNER_FRAMES[state.frame])} ${
          many ? theme.fg("dim", `#${r.seq ?? 1}`) + " " : ""
        }${t}`,
		);
	}
	if (state.local) {
		return [`${theme.fg("accent", SPINNER_FRAMES[state.frame])} ${state.local}`];
	}
	if (state.lastText) {
		// 流停了：最后一行速率前加「history:」前缀，颜色照旧（不灰显）。
		return [`history: ${state.lastText}`];
	}
	if (state.updatedAt) {
		return [`${theme.fg("dim", "● 空闲")} ${theme.fg("dim", `llm-switch · ${APP}`)}`];
	}
	return [];
}

function flush(ctx: ExtensionContext): void {
	if (!enabled || ctx.mode !== "tui" || !ctx.hasUI) return;
	const lines = buildContent(ctx.ui.theme);
	// 空内容传 undefined：pi 那边就是「撤掉这条 widget」，而不是留一个空框。
	ctx.ui.setWidget("llm-switch-rate", lines.length ? lines : undefined);
}

async function tick(ctx: ExtensionContext): Promise<void> {
	const v = await fetchLine();
	if (v && (v.rows?.length ?? 0) > 0) {
		// 网关有行：全文照抄，本地估算靠边站。
		state.rows = v.rows;
		state.updatedAt = Date.now();
		state.local = "";
	} else {
		state.rows = [];
		state.updatedAt = v ? Date.now() : 0;
		// 网关活着但没行（直连在线 API / 本代理没挂号）：落本地估算。
		state.local = localText(Date.now());
	}
	// 记住最近一次在飞的速率行（裸文案，不带主题色），流停了在它前面加「history:」前缀。
	const live = state.local || plainLive();
	if (live) state.lastText = live;
	flush(ctx);
}

/** 网关行里第一条能显示 decode 的裸文案（不带主题色，供「history:」复用）。 */
function plainLive(): string {
	const rows = [...state.rows].sort((a, b) => (PHASE_RANK[a.phase] ?? 9) - (PHASE_RANK[b.phase] ?? 9));
	for (const r of rows) {
		if (r.phase === "queued" || r.phase === "prefill") continue;
		return `▶ ${tps(r.decode_tps)} tok/s`;
	}
	return "";
}

export default function llmSwitchRateExtension(pi: ExtensionAPI): void {
	pi.on("session_start", (_event, ctx) => {
		if (!ctx.hasUI || ctx.mode !== "tui") return;
		flush(ctx);
		void tick(ctx);
		if (!state.timer) state.timer = setInterval(() => void tick(ctx), POLL_MS);
	});

	pi.on("session_shutdown", () => {
		if (state.timer) {
			clearInterval(state.timer);
			state.timer = 0;
		}
	});

	// 本地估算的记账：消息流事件只更新账本，rate 都在 tick 里现算。
	pi.on("message_start", (event) => {
		try {
			onMessageStart((event as any).message);
		} catch {
			// 记账出错不许影响显示。
		}
	});
	pi.on("message_update", (event) => {
		try {
			onMessageUpdate((event as any).assistantMessageEvent);
		} catch {
			// 跳过这一个事件。
		}
	});
	pi.on("message_end", (event) => {
		try {
			onMessageEnd((event as any).message);
		} catch {
			// 跳过这一个事件。
		}
	});

	pi.registerCommand("llm-switch-rate", {
		description: "显示/隐藏 llm-switch 速率条（数值与浮窗同源）。",
		handler: async (args, ctx) => {
			const arg = args.trim().toLowerCase();
			if (arg === "off") {
				enabled = false;
				if (ctx.hasUI && ctx.mode === "tui") ctx.ui.setWidget("llm-switch-rate", undefined);
				ctx.ui.notify("llm-switch-rate：状态条已隐藏", "info");
			} else if (arg === "on") {
				enabled = true;
				if (ctx.hasUI && ctx.mode === "tui") void tick(ctx);
				ctx.ui.notify("llm-switch-rate：状态条已显示", "info");
			} else {
				ctx.ui.notify(
					`llm-switch-rate 当前${enabled ? "显示中" : "已隐藏"}；用 /llm-switch-rate on 或 off 切换`,
					"info",
				);
			}
		},
	});
}
