/**
 * 首次引导 Tour（借鉴 unsloth studio features/tour）：首启自动弹一次，逐步介绍主界面，
 * 可跳过；「已看过」记 localStorage。设置页或快捷键可经全局事件 `llmswitch:tour` 重放。
 */
import { useEffect, useRef, useState } from "react";
import { useEscape, useFocusTrap } from "./a11y";
import { t, ts } from "../lib/i18n";

const SEEN_KEY = "llmswitch.tour.seen";

interface Step {
  titleKey: string;
  bodyKey: string;
}

const STEPS: Step[] = [
  { titleKey: "gt.s1t", bodyKey: "gt.s1b" },
  { titleKey: "gt.s2t", bodyKey: "gt.s2b" },
  { titleKey: "gt.s3t", bodyKey: "gt.s3b" },
  { titleKey: "gt.s4t", bodyKey: "gt.s4b" },
  { titleKey: "gt.s5t", bodyKey: "gt.s5b" },
];

export function GuidedTour() {
  const [open, setOpen] = useState(false);
  const [i, setI] = useState(0);
  const panelRef = useRef<HTMLDivElement>(null);

  const close = () => {
    localStorage.setItem(SEEN_KEY, "1");
    setOpen(false);
  };
  const next = () => {
    if (i >= STEPS.length - 1) close();
    else setI(i + 1);
  };
  const prev = () => setI((x) => Math.max(0, x - 1));

  // 首启自动弹一次。
  useEffect(() => {
    if (!localStorage.getItem(SEEN_KEY)) {
      setOpen(true);
      setI(0);
    }
  }, []);

  // 允许外部经全局事件重放（设置页「重看引导」用）。
  useEffect(() => {
    const onReplay = () => {
      setI(0);
      setOpen(true);
    };
    window.addEventListener("llmswitch:tour", onReplay);
    return () => window.removeEventListener("llmswitch:tour", onReplay);
  }, []);

  // 模态口径补齐（U10）：此前这块浮层能开不能关——没有 Esc、焦点能从 Tab 溜到背后页面上。
  useEscape(close, open);
  useFocusTrap(panelRef, open);

  if (!open) return null;

  const step = STEPS[i];

  return (
    <div
      className="fixed inset-0 z-[100] grid place-items-center bg-black/60 p-4"
      onClick={close}
    >
      <div
        ref={panelRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby="tour-title"
        className="w-full max-w-md rounded-xl border border-slate-700 bg-slate-900 p-6 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="mb-2 text-xs text-ink-4">
          {ts("gt.welcome", { i: i + 1, n: STEPS.length })}
        </div>
        <h2 id="tour-title" className="mb-2 text-lg font-semibold text-ink-1">
          {t(step.titleKey)}
        </h2>
        <p className="mb-6 text-sm leading-relaxed text-ink-2">{t(step.bodyKey)}</p>

        <div className="mb-5 flex gap-1.5">
          {STEPS.map((_, k) => (
            <span
              key={k}
              className={`h-1.5 flex-1 rounded-full ${k === i ? "bg-cyan-400" : "bg-slate-700"}`}
            />
          ))}
        </div>

        <div className="flex items-center justify-between">
          <button
            onClick={close}
            className="text-xs text-ink-4 transition-colors hover:text-ink-2"
          >
            {t("gt.skip")}
          </button>
          <div className="flex gap-2">
            {i > 0 && (
              <button
                onClick={prev}
                className="rounded-lg border border-slate-700 px-3 py-1.5 text-sm text-ink-2 transition-colors hover:border-slate-600"
              >
                {t("gt.prev")}
              </button>
            )}
            <button
              onClick={next}
              className="rounded-lg bg-cyan-500 px-4 py-1.5 text-sm font-medium text-slate-950 transition-colors hover:bg-cyan-400"
            >
              {i >= STEPS.length - 1 ? t("gt.start") : t("gt.next")}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

/** 供其它页面调用：重放引导。 */
export function replayTour() {
  window.dispatchEvent(new Event("llmswitch:tour"));
}
