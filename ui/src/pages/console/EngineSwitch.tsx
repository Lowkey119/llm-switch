// 引擎面板视图切换（配对后端才渲染）：点另一个引擎**只切右侧面板看谁**，
// 不碰任何引擎——用户口径：切页面 ≠ 切引擎。真正的互斥切换（先停在跑的 →
// 起目标）发生在目标面板里按下「启动」时（HalogenPanel 的互斥启动确认）。
// engine = 当前生效引擎（打点标记），view = 当前在看的面板（高亮段）。
import { ipc, type HalStatus } from "../../lib/ipc";
import { useEffect, useState } from "react";

export function EngineSwitch({
  engine,
  alt,
  view,
  onView,
}: {
  engine: string;
  alt: string;
  view: string;
  onView: (ch: string) => void;
}) {
  // 对面引擎是否真在跑（生效引擎列可能滞后，绿点以引擎实际状态为准）。
  const other = view === engine ? alt : engine;
  const [otherRunning, setOtherRunning] = useState(false);
  useEffect(() => {
    let dead = false;
    const poll = async () => {
      try {
        const s = await ipc<HalStatus>("halctl_status", { ch: other });
        if (!dead) setOtherRunning(Boolean(s.running));
      } catch {
        if (!dead) setOtherRunning(false);
      }
    };
    void poll();
    const t = setInterval(() => void poll(), 5000);
    return () => {
      dead = true;
      clearInterval(t);
    };
  }, [other]);

  return (
    <span className="flex items-center gap-1.5 text-xs text-ink-4">
      引擎
      <span className="inline-flex overflow-hidden rounded-md border border-slate-700">
        {[engine, alt].map((e) => (
          <button
            key={e}
            onClick={() => e !== view && onView(e)}
            title={`查看 ${e} 面板（不切换引擎；要换引擎去该面板里按启动）`}
            className={`flex items-center gap-1 px-2 py-0.5 transition-colors ${
              e === view
                ? "bg-cyan-500/20 font-medium text-cyan-300"
                : "text-ink-4 hover:bg-slate-800"
            }`}
          >
            {e}
            {e === engine || (e === other && otherRunning) ? (
              <span
                className="inline-block h-1.5 w-1.5 rounded-full bg-emerald-400"
                title="该引擎正在运行"
              />
            ) : null}
          </button>
        ))}
      </span>
      {view !== engine && <span className="text-2xs text-ink-4">（查看中，未切换）</span>}
    </span>
  );
}
