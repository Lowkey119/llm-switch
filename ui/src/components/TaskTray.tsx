/**
 * 后台任务托盘（借鉴 unsloth studio 后台任务 + 完成通知）：顶栏一枚带未读红点的按钮，
 * 点开看各长操作进度；任务从 Running 转 Done/Failed 时弹 toast。
 *
 * 沿用前端既有轮询口径（useQuery + refetchInterval），不引 Tauri 事件监听。
 */
import { useEffect, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useToast } from "./Toast";
import { IconSpinner } from "./icons";
import { ipc } from "../lib/ipc";
import { every, POLL } from "../lib/poll";
import { t, ts } from "../lib/i18n";

type TaskStatus = "running" | "done" | "failed";
interface Task {
  id: string;
  kind: string;
  label: string;
  status: TaskStatus;
  progress: number;
  message: string;
  started_at: number;
  finished_at: number | null;
}

/** 状态标签存 i18n key，取词处 t()。 */
const STATUS_LABEL: Record<TaskStatus, string> = {
  running: "tt.running",
  done: "tt.done",
  failed: "tt.failed",
};

export function TaskTray() {
  const qc = useQueryClient();
  const toast = useToast();
  const [open, setOpen] = useState(false);

  const tasks = useQuery({
    queryKey: ["tasks"],
    queryFn: () => ipc<Task[]>("list_tasks"),
    refetchInterval: () => every(POLL.fast),
  });
  const unread = useQuery({
    queryKey: ["task-unread"],
    queryFn: () => ipc<number>("task_unread"),
    refetchInterval: () => every(POLL.fast),
  });

  // 完成通知：对比上一轮状态，Running→Done/Failed 的那几条弹 toast（每个 id 只弹一次）。
  const seenDone = useRef<Set<string>>(new Set());
  const prevStatus = useRef<Map<string, TaskStatus>>(new Map());
  useEffect(() => {
    const list = tasks.data ?? [];
    for (const tk of list) {
      const before = prevStatus.current.get(tk.id);
      if (tk.status !== "running" && before === "running" && !seenDone.current.has(tk.id)) {
        seenDone.current.add(tk.id);
        const msg = tk.message
          ? ts("tt.noteWith", { label: tk.label, status: t(STATUS_LABEL[tk.status]), msg: tk.message })
          : ts("tt.note", { label: tk.label, status: t(STATUS_LABEL[tk.status]) });
        if (tk.status === "failed") toast.err(msg);
        else toast.ok(msg);
      }
      prevStatus.current.set(tk.id, tk.status);
    }
  }, [tasks.data, toast]);

  const n = unread.data ?? 0;
  const running = (tasks.data ?? []).filter((tk) => tk.status === "running").length;

  const toggle = () => {
    const next = !open;
    setOpen(next);
    if (next && n > 0) {
      void ipc("mark_tasks_seen").then(() => {
        qc.invalidateQueries({ queryKey: ["task-unread"] });
      });
    }
  };

  const list = tasks.data ?? [];
  // tooltip 要能回答「现在有没有后台消息」：光一枚齿轮看不出来在跑几条、有没有没看的。
  const tip =
    running > 0
      ? ts("top.tasksRunning", { n: running })
      : n > 0
        ? ts("top.tasksUnread", { n })
        : t("top.tasksIdle");

  return (
    <div className="relative">
      <button
        data-tip={ts(running > 0 ? "top.tasksRunning" : n > 0 ? "top.tasksUnread" : "top.tasksIdle", { n })}
        aria-label={tip}
        onClick={toggle}
        className={`tip tip-edge relative grid h-8 w-8 place-items-center rounded-lg border border-slate-700 bg-slate-800/60 transition-colors hover:border-cyan-500/50 hover:text-cyan-300 ${
          running > 0 ? "text-cyan-300" : "text-ink-3"
        }`}
      >
        {/* 有任务在跑时转圈。原先是这里手搓的一份 SVG，笔画 2 与全族 1.7 对不上，
            收进 icons.tsx 走同一份 base()。 */}
        <IconSpinner width={16} height={16} className={running > 0 ? "animate-spin" : ""} />
        {n > 0 && (
          <span className="absolute -right-1 -top-1 grid h-4 min-w-4 place-items-center rounded-full bg-rose-500 px-1 text-2xs font-semibold text-white">
            {n}
          </span>
        )}
      </button>

      {open && (
        <div className="absolute right-0 top-9 z-50 w-80 rounded-lg border border-slate-700 bg-slate-900 p-2 shadow-xl">
          <div className="mb-1 flex items-center justify-between px-1 text-xs text-ink-4">
            <span>{t("tt.title")}</span>
            <span>{running > 0 ? ts("tt.runningN", { n: running }) : t("tt.noneRunning")}</span>
          </div>
          {list.length === 0 ? (
            <p className="px-1 py-3 text-center text-xs text-ink-4">{t("tt.empty")}</p>
          ) : (
            <ul className="max-h-80 space-y-1.5 overflow-auto">
              {list.map((tk) => (
                <li key={tk.id} className="rounded-md bg-slate-800/50 px-2 py-1.5">
                  <div className="flex items-center justify-between gap-2 text-xs">
                    <span className="truncate text-ink-1" title={tk.label}>{tk.label}</span>
                    <span
                      className={
                        tk.status === "failed"
                          ? "text-[var(--sem-danger-text)]"
                          : tk.status === "done"
                            ? "text-[var(--sem-ok-text)]"
                            : "text-cyan-300"
                      }
                    >
                      {t(STATUS_LABEL[tk.status])}
                    </span>
                  </div>
                  {tk.status === "running" && (
                    <div className="mt-1 h-1 w-full overflow-hidden rounded bg-slate-700">
                      <div
                        className="h-full bg-cyan-400 transition-all"
                        style={{ width: `${Math.round(tk.progress * 100)}%` }}
                      />
                    </div>
                  )}
                  {tk.message && (
                    <div className="mt-1 truncate text-xs text-ink-4" title={tk.message}>
                      {tk.message}
                    </div>
                  )}
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </div>
  );
}
