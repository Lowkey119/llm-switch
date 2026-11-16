/**
 * 首次引导 Tour（借鉴 unsloth studio features/tour）：首启自动弹一次，逐步介绍主界面，
 * 可跳过；「已看过」记 localStorage。设置页或快捷键可经全局事件 `llmswitch:tour` 重放。
 */
import { useEffect, useState } from "react";

const SEEN_KEY = "llmswitch.tour.seen";

interface Step {
  title: string;
  body: string;
}

const STEPS: Step[] = [
  {
    title: "实时监控",
    body: "看此刻在飞的请求：prefill / decode 速率、TTFT、已用上下文。速率优先取引擎实测真值，取不到时用纠偏系数估算。",
  },
  {
    title: "后端与模型",
    body: "聚合的多机推理服务在这里统一登记。模型清单、参数中心、缓存策略都从后端页进入。",
  },
  {
    title: "代理接管",
    body: "一键把 Claude Code / Codex 等客户端接到本地网关，自动备份原配置、可还原。",
  },
  {
    title: "命令面板",
    body: "按 ⌘K / Ctrl+K 随时唤起，直接跳页面或执行动作，不必回回先动鼠标。",
  },
  {
    title: "后台任务",
    body: "顶栏的任务图标汇总安装、接管、后端更新等长操作，完成会弹通知并累计未读红点。",
  },
];

export function GuidedTour() {
  const [open, setOpen] = useState(false);
  const [i, setI] = useState(0);

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

  if (!open) return null;

  const close = () => {
    localStorage.setItem(SEEN_KEY, "1");
    setOpen(false);
  };
  const next = () => {
    if (i >= STEPS.length - 1) close();
    else setI(i + 1);
  };
  const prev = () => setI((x) => Math.max(0, x - 1));

  const step = STEPS[i];

  return (
    <div
      className="fixed inset-0 z-[100] grid place-items-center bg-black/60 p-4"
      onClick={close}
    >
      <div
        className="w-full max-w-md rounded-xl border border-slate-700 bg-slate-900 p-6 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="mb-2 text-xs text-ink-4">
          欢迎使用 · {i + 1} / {STEPS.length}
        </div>
        <h2 className="mb-2 text-lg font-semibold text-ink-1">{step.title}</h2>
        <p className="mb-6 text-sm leading-relaxed text-ink-2">{step.body}</p>

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
            跳过引导
          </button>
          <div className="flex gap-2">
            {i > 0 && (
              <button
                onClick={prev}
                className="rounded-lg border border-slate-700 px-3 py-1.5 text-sm text-ink-2 transition-colors hover:border-slate-600"
              >
                上一步
              </button>
            )}
            <button
              onClick={next}
              className="rounded-lg bg-cyan-500 px-4 py-1.5 text-sm font-medium text-slate-950 transition-colors hover:bg-cyan-400"
            >
              {i >= STEPS.length - 1 ? "开始使用" : "下一步"}
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
