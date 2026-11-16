/**
 * 后台任务托盘（借鉴 unsloth studio 后台任务 + 完成通知）：顶栏一枚带未读红点的按钮，
 * 点开看各长操作进度；任务从 Running 转 Done/Failed 时弹 toast。
 *
 * 沿用前端既有轮询口径（useQuery + refetchInterval），不引 Tauri 事件监听。
 */
import { useEffect, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useToast } from "./Toast";
import { ipc } from "../lib/ipc";
import { every, POLL } from "../lib/poll";

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

const STATUS_LABEL: Record<TaskStatus, string> = {
  running: "进行中",
  done: "完成",
  failed: "失败",
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
    for (const t of list) {
      const before = prevStatus.current.get(t.id);
      if (t.status !== "running" && before === "running" && !seenDone.current.has(t.id)) {
        seenDone.current.add(t.id);
        const msg = `${t.label} ${STATUS_LABEL[t.status]}${t.message ? "：" + t.message : ""}`;
        if (t.status === "failed") toast.err(msg);
        else toast.ok(msg);
      }
      prevStatus.current.set(t.id, t.status);
    }
  }, [tasks.data, toast]);

  const n = unread.data ?? 0;
  const running = (tasks.data ?? []).filter((t) => t.status === "running").length;

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

  return (
    <div className="relative">
      <button
        title="后台任务"
        aria-label="后台任务"
        onClick={toggle}
        className={`relative grid h-8 w-8 place-items-center rounded-lg border border-slate-700 bg-slate-800/60 transition-colors hover:border-cyan-500/50 hover:text-cyan-300 ${
          running > 0 ? "text-cyan-300" : "text-ink-3"
        }`}
      >
        {/* 齿轮状占位图标：有任务在跑时转圈。 */}
        <svg
          width={16}
          height={16}
          viewBox="0 0 24 24"
          fill="none"
          className={running > 0 ? "animate-spin" : ""}
        >
          <circle cx="12" cy="12" r="9" stroke="currentColor" strokeWidth="2" opacity="0.25" />
          <path d="M21 12a9 9 0 0 0-9-9" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
        </svg>
        {n > 0 && (
          <span className="absolute -right-1 -top-1 grid h-4 min-w-4 place-items-center rounded-full bg-rose-500 px-1 text-2xs font-semibold text-white">
            {n}
          </span>
        )}
      </button>

      {open && (
        <div className="absolute right-0 top-9 z-50 w-80 rounded-lg border border-slate-700 bg-slate-900 p-2 shadow-xl">
          <div className="mb-1 flex items-center justify-between px-1 text-xs text-ink-4">
            <span>后台任务</span>
            <span>{running > 0 ? `${running} 进行中` : "无进行中"}</span>
          </div>
          {list.length === 0 ? (
            <p className="px-1 py-3 text-center text-xs text-ink-4">暂无后台任务</p>
          ) : (
            <ul className="max-h-80 space-y-1.5 overflow-auto">
              {list.map((t) => (
                <li key={t.id} className="rounded-md bg-slate-800/50 px-2 py-1.5">
                  <div className="flex items-center justify-between gap-2 text-xs">
                    <span className="truncate text-ink-1" title={t.label}>{t.label}</span>
                    <span
                      className={
                        t.status === "failed"
                          ? "text-[var(--sem-danger-text)]"
                          : t.status === "done"
                            ? "text-[var(--sem-ok-text)]"
                            : "text-cyan-300"
                      }
                    >
                      {STATUS_LABEL[t.status]}
                    </span>
                  </div>
                  {t.status === "running" && (
                    <div className="mt-1 h-1 w-full overflow-hidden rounded bg-slate-700">
                      <div
                        className="h-full bg-cyan-400 transition-all"
                        style={{ width: `${Math.round(t.progress * 100)}%` }}
                      />
                    </div>
                  )}
                  {t.message && (
                    <div className="mt-1 truncate text-xs text-ink-4" title={t.message}>
                      {t.message}
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
