// 浮动面板（M12-C2a/C2b）：不看主窗也能瞄一眼「此刻在飞什么」。
//
// 数据只有一个来源：`get_rates`（gateway/rates.rs 的统一出口），和监控页那张
// 表、REST /admin/rates 完全同源——两处口径不一致是最难查的 bug，宁可少画一格。
// 速率列切片 11 起改吃引擎原生速率（get_engine_rates 经 backend→engine 解析）。
//
// 纪律：没有真值的列一律不画；引擎没报速率显示 —，估算补位是红线。
//
// 交互对齐参考实现 tokbar：任意位置拖动、双击在「4 行 ⇄ 多行」之间切换
// （多行上限 12 行，高度再高由 Rust 侧钳到屏幕 70%；圆钮态双击展开成面板并
// 恢复折叠前的行数模式）、拖到左右边缘缩成贴边圆钮、Cmd+滚轮/Cmd±0 缩放。
// 形状的真值在 Rust 侧（那边才知道屏幕工作区），这里只渲染与回报尺寸。
//
// 单行速率线不在这个窗口里：它是每代理一个独立窗口（见 FloatRateLine.tsx），
// 本窗口永远是全局的多行面板。

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { ipc } from "../lib/ipc";
import { useZoom } from "../lib/zoom";
import { every, POLL } from "../lib/poll";
import { shortModel } from "../lib/modelprefix";
import { formatVersion, useAppVersion } from "../lib/version";
import { IconClose } from "./icons";
import {
  engineMap,
  makeNativeResolver,
  useBackendEngines,
  useEngineRates,
} from "../lib/nativeRates";
import { headline } from "./FloatRateLine";
import type { Inflight } from "./FloatRateLine";

interface RatesSnapshot {
  inflight: Inflight[];
}

interface Shape {
  expanded: boolean;
  edge: number;
  width: number;
  height: number;
  show_all: boolean;
}

/** 展开态默认只显示前 N 行；双击展开多行（tokbar 的 multiRow 同款），
 * 行数自适应但封顶 MAX_ROWS——行数无上限会把面板撑出屏幕。 */
const DEFAULT_ROWS = 4;
const MAX_ROWS = 12;

function Orb() {
  return (
    <svg viewBox="0 0 24 24" className="h-6 w-6" fill="none" stroke="currentColor" strokeWidth={2}>
      <path
        d="M2 12h3l2-6 3 12 3-9 2 3h7"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

export default function FloatPanel() {
  const [shape, setShape] = useState<Shape | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  // 浮窗身份行带上版本号（UI-AUDIT-2 F-1）：飘在桌面上那颗窗也能说清自己是哪一版。
  const version = formatVersion(useAppVersion());
  // 上一次按下的时刻，用来自己判双击（见 press）
  const lastPress = useRef(0);
  /** 本次按下是否已经“动起来”（超过阈值就交给原生拖拽）。 */
  const armed = useRef<{ x: number; y: number; moved: boolean } | null>(null);

  const rates = useQuery<RatesSnapshot>({
    queryKey: ["float-rates"],
    queryFn: () => ipc<RatesSnapshot>("get_rates"),
    refetchInterval: () => every(POLL.live),
  });
  // 引擎原生速率（切片 11）：与监控页/速率线同源，后端 TTL 合流。
  const er = useEngineRates(POLL.live);
  const engines = useBackendEngines();
  const nativeFor = useMemo(
    () => makeNativeResolver(er.data, engineMap(engines.data)),
    [er.data, engines.data],
  );

  // 形状：初始拉一次，之后靠 Rust 侧的事件同步（双击、贴边都发生在那边）。
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    ipc<Shape>("float_state")
      .then((s) => !cancelled && setShape(s))
      .catch(() => {});
    import("@tauri-apps/api/event")
      .then(({ listen }) =>
        listen<Shape>("float://shape", (ev) => !cancelled && setShape(ev.payload)),
      )
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  /** 把窗口收到内容的实际大小；`center` 时以中心为锚点（缩放用）。
   * 拖动中的 fit 不在这里拦——拦了就永远丢掉，等不到松手。Rust 侧
   * `fit_or_defer` 记账并等拖动静默后落最新一笔，唯一出口在那边。
   *
   * 测量**不要乘 zoom**：当前 WebKit 的 `getBoundingClientRect()` 已经含
   * documentElement 的 CSS zoom（诊断日志实测：zoom=2.069 时量出
   * 480×z²=2057，正中上报值）。再乘一次会放大成 z²——窗口比内容宽一倍
   * 留出大片空区，更糟的是窗口宽过工作区会把左右贴边裁决整个禁掉
   * （floatwin 的 fits），贴边的 x 钳制还会退化成把窗口甩到左边缘。 */
  const refit = useCallback((center: boolean) => {
    const el = rootRef.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    if (r.width < 8 || r.height < 8) return;
    const cmd = center ? "float_zoom_resize" : "float_fit";
    void ipc(cmd, {
      width: Math.ceil(r.width) + 2,
      height: Math.ceil(r.height) + 2,
    }).catch(() => {});
  }, []);

  // 缩放：倍率变了先重排，再把窗口尺寸按新大小收回来（中心不动）。
  // 注意倍率只作用于展开的面板；贴边那颗圆钮始终保持标准大小（见下面的反缩放）。
  const zoom = useZoom("float", (_z) => {
    if (shape?.expanded !== false) refit(true);
  });

  const rows = rates.data?.inflight ?? [];
  const busy = rows.length > 0;
  // show_all（多行态）：双击展开显示至多 MAX_ROWS 行，再双击收回前 DEFAULT_ROWS 行。
  const shown = rows.slice(0, shape?.show_all ? MAX_ROWS : DEFAULT_ROWS);
  const rest = rows.length - shown.length;

  // 行数变化会让内容变高，窗口跟着收（差得太小就不折腾，免得来回抖）。
  useEffect(() => {
    if (shape?.expanded === false) return;
    const el = rootRef.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    if (
      shape &&
      (Math.abs(r.width - shape.width) > 3 || Math.abs(r.height - shape.height) > 3)
    ) {
      refit(false);
    }
  }, [rows.length, shape, refit]);

  // Esc 关掉，省得为了关闭去抓鼠标。
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") void ipc("float_set_visible", { visible: false });
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  /**
   * 整个窗口都是拖拽区（跟 tokbar 一样没有小把手），但按下不马上开拖——
   * macOS 的 `startDragging()` 会把 mouseUp 连同下一次 down 一口吞掉，
   * 双击的第二下就永远到不了 JS（之前“双击打不开”就是这个）。
   *
   * 所以现在：按下只记账；手指真动了（>4px）才交给原生拖拽——位移比肉眼快，
   * 手感上仍是“按住就走”。两次按下隔得不近就算双击（`e.detail` 在 WKWebView
   * 里不保证是点击次数，所以自己计时间戳）。
   */
  const press = (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    const now = performance.now();
    const dbl = e.detail >= 2 || now - lastPress.current < 420;
    lastPress.current = now;
    if (dbl) {
      lastPress.current = 0;
      armed.current = null;
      void ipc<Shape>("float_double_click").then(setShape).catch(() => {});
      return;
    }
    armed.current = { x: e.clientX, y: e.clientY, moved: false };
  };

  /** 起手超过阈值才开拖；拖过就不再参与双击计数。 */
  const drift = (e: React.PointerEvent) => {
    const a = armed.current;
    if (!a || a.moved) return;
    if (Math.hypot(e.clientX - a.x, e.clientY - a.y) < 4) return;
    a.moved = true;
    lastPress.current = 0;
    void getCurrentWindow()
      .startDragging()
      .catch(() => {});
  };

  // 圆钮态：只剩一颗贴边的钮，双击展开。
  if (shape && !shape.expanded) {
    return (
      <div
        ref={rootRef}
        className="float-orb select-none"
        // CSS zoom 是会往下传的：面板缩到 0.7 时这颗钮也会跟着变小，可窗口仍是
        // BUTTON 那么大，看着就是一颗缩在角落的小图标。这里乘个倒数抵回去，
        // 圆钮无论何时都是标准尺寸（窗口本来就是按 BUTTON 开的）。
        style={{ zoom: 1 / zoom }}
        onPointerDown={press}
        onPointerMove={drift}
        onPointerUp={() => {
          armed.current = null;
        }}
        onPointerCancel={() => {
          armed.current = null;
        }}
        title="llm-switch 浮窗：双击展开，拖动可贴边"
      >
        <Orb />
      </div>
    );
  }

  return (
    <div
      ref={rootRef}
      className={`float-card group/float select-none ${busy ? "" : "float-idle"}`}
      onPointerDown={press}
      onPointerMove={drift}
      onPointerUp={() => {
        armed.current = null;
      }}
      onPointerCancel={() => {
        armed.current = null;
      }}
    >
      <div className="flex items-center justify-between gap-2 text-2xs leading-none text-ink-3">
        <span className="flex items-center gap-1.5 tracking-wide">
          llm-switch
          <span className="font-mono text-ink-4">{version}</span>
        </span>
        <span className="flex items-center gap-1.5">
          <span className="tabular-nums">
            {busy ? (
              // 状态指示器：双击切换行数模式时这里会明显变化，便于肉眼确认生效。
              // 多行态没截断标「（全部）」；被 4 行/12 行截断时显示「已显/总数」。
              shape?.show_all && rows.length <= MAX_ROWS ? (
                <>在飞 {rows.length}（全部）</>
              ) : rows.length > shown.length ? (
                <>在飞 {shown.length}/{rows.length}</>
              ) : (
                <>在飞 {rows.length}</>
              )
            ) : (
              "空闲"
            )}
          </span>
          {/* 收起入口（UI-AUDIT-2 D-2）：以前这张卡面上没有任何关闭控件，
              只能 Esc 或拖到贴边——资源窗有 ✕，三扇浮窗该是同一手感。
              悬停才显形，不抢常驻视觉；onPointerDown 必须 stopPropagation，
              否则会被整窗拖拽/双击判定吞掉。 */}
          <button
            type="button"
            title="隐藏浮窗（Esc）"
            aria-label="隐藏浮窗"
            onPointerDown={(e) => e.stopPropagation()}
            onClick={() => void ipc("float_set_visible", { visible: false })}
            className="grid h-4 w-4 place-items-center rounded text-ink-4 opacity-0 transition-[opacity,color] hover:bg-slate-700/60 hover:text-ink-1 focus-visible:opacity-100 group-hover/float:opacity-100"
          >
            <IconClose width={10} height={10} strokeWidth={2.4} />
          </button>
        </span>
      </div>

      {!busy && (
        <div className="mt-1.5 text-xs text-ink-4">
          {rates.isError ? "取不到网关数据" : "没有在飞的请求"}
        </div>
      )}

      {shown.map((s) => {
        const h = headline(s, nativeFor(s.backend, s.model));
        // 行格式（用户口径）：后端 · 调用代理 > 暴露模型（别名） · 消息类型。
        // 认不出代理时只省略「代理 >」段，不猜不编。
        const who = s.app ? `${s.app} > ${s.model ? shortModel(s.model) : "未知模型"}` : (s.model ? shortModel(s.model) : "未知模型");
        return (
          <div key={s.id} className="mt-1.5 leading-tight">
            <div className="flex items-baseline gap-1.5">
              <span className={s.phase === "generating" ? "text-[var(--sem-ok-text)]" : "text-cyan-400"}>
                {h.icon}
              </span>
              <span className="tabular-nums text-xs text-ink-1">{h.text}</span>
            </div>
            {/* 面板定宽 480，长模型 id 必截断：悬停给回完整原文（E-1）。 */}
            <div
              className="truncate pl-5 text-2xs text-ink-3"
              title={`${s.backend}${s.app ? ` · ${s.app}` : ""} · ${s.model ?? "未知模型"}${s.entry ? ` · ${s.entry}` : ""}`}
            >
              {[s.backend, who, s.entry].filter(Boolean).join(" · ")}
            </div>
          </div>
        );
      })}

      {rest > 0 && <div className="mt-1 pl-5 text-xs text-ink-4">还有 {rest} 条…</div>}
    </div>
  );
}
