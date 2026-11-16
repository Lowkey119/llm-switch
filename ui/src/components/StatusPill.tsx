// 顶栏代理状态胶囊（从 App.tsx 原样搬出）：连接中 / 已暂停 / 运行中三态，
// 含统一转发端口的原位编辑（校验链 + 占用探测 + 二次确认，决策 D1、任务 3.2）。
import { useEffect, useRef, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useLive } from "../lib/live";
import { StatusDot } from "./ui";
import { useToast } from "./Toast";
import { ConfirmDialog, Risk } from "./ConfirmDialog";
import { IconPencil } from "./icons";
import { ipc, type ShellStatus } from "../lib/ipc";

function fmtTokens(n: number) {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}

export function StatusPill({ status, error }: { status?: ShellStatus; error: boolean }) {
  // 顶栏那颗点也跟流量走：有请求在飞才呼吸，没请求就老实亮绿。
  const live = useLive(1500);
  if (error || !status) {
    return (
      <span className="flex h-8 items-center gap-2 rounded-full border border-slate-700 bg-slate-800/60 px-3 text-xs text-ink-3">
        连接内核中…
      </span>
    );
  }
  if (status.paused) {
    return (
      <span className="flex h-8 items-center gap-2 rounded-full border border-amber-500/40 bg-amber-500/10 px-3 text-xs text-[var(--sem-busy-text)]">
        <span className="h-1.5 w-1.5 rounded-full bg-amber-400" />
        已暂停接单 · {status.listen}
        {/* 窄窗口（<lg）先丢「今日 tokens」这段次要信息，保住端口与状态本体（UI 审查 P2-9） */}
        <span className="hidden lg:inline"> · 今日 {fmtTokens(status.today_tokens)} tokens</span>
      </span>
    );
  }
  return (
    <span className="flex h-8 items-center gap-2 rounded-full border border-emerald-500/30 bg-emerald-500/10 px-3 text-xs text-[var(--sem-ok-text)]">
      <StatusDot busy={live.total > 0} />
      运行中 · <ListenAddr listen={status.listen} />
      <span className="hidden lg:inline"> · 今日 {fmtTokens(status.today_tokens)} tokens</span>
    </span>
  );
}

/**
 * 统一转发端口（UI 重设计 §七点五）：原位编辑，校验链 + 占用探测 + 二次确认。
 *
 * 探测和确认只由「Enter / 应用」触发（决策 D1、任务 3.2）：失焦一律静默放弃。
 * 原先失焦即跑探测并弹确认框，用户点个别处就被问一次「要不要换端口」，
 * 而重启会掐断在飞请求 —— 这种后果必须只跟着明确意图。
 */
function ListenAddr({ listen }: { listen: string }) {
  const qc = useQueryClient();
  const toast = useToast();
  const idx = listen.lastIndexOf(":");
  const host = listen.slice(0, idx);
  const curPort = Number(listen.slice(idx + 1));
  const [edit, setEdit] = useState(false);
  const [val, setVal] = useState("");
  const [err, setErr] = useState<string | null>(null);
  const [confirmPort, setConfirmPort] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);
  const errTimer = useRef<number | null>(null);

  // 卸载/连点都要清掉上一条气泡的定时器，否则旧定时器会提前抹掉新提示。
  useEffect(
    () => () => {
      if (errTimer.current) window.clearTimeout(errTimer.current);
    },
    [],
  );

  const apply = useMutation({
    mutationFn: (p: number) => ipc<string>("set_listen_port", { port: p }),
    onSuccess: (listenNew) => {
      // 乐观更新胶囊（status 轮询 5s 才回，不等它）；后台失败会在下次轮询里纠偏。
      qc.setQueryData(["status"], (old: ShellStatus | undefined) =>
        old ? { ...old, listen: listenNew } : old,
      );
      qc.invalidateQueries({ queryKey: ["status"] });
    },
    onError: (e) => toast.err(`改端口失败：${e}`),
    onSettled: () => setBusy(false),
  });

  const flashErr = (m: string) => {
    setErr(m);
    if (errTimer.current) window.clearTimeout(errTimer.current);
    errTimer.current = window.setTimeout(() => setErr(null), 3000);
  };

  /** 校验 → 占用探测 → 二次确认。出错时留在编辑态，让用户就地改而不是重敲。 */
  const commit = async () => {
    const v = val.trim();
    if (!v) return setEdit(false); // 空值 = 放弃
    const p = Number(v);
    if (!/^\d+$/.test(v) || p < 1 || p > 65535) return flashErr("端口需为 1–65535 的数字");
    if (p === curPort) return setEdit(false); // 没动就不打扰
    try {
      const free = await ipc<boolean>("check_port", { port: p });
      if (!free) return flashErr(`端口 ${p} 已被占用`);
    } catch (e) {
      return flashErr(`占用探测失败：${e}`);
    }
    setEdit(false);
    setConfirmPort(p);
  };

  if (busy) {
    return <span className="text-ink-3">重启中…</span>;
  }
  return (
    <span className="relative inline-flex items-center">
      {edit ? (
        <>
          {host}:
          <input
            autoFocus
            value={val}
            placeholder={String(curPort)}
            aria-label="统一转发端口"
            onChange={(e) => {
              setVal(e.target.value);
              setErr(null);
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter") void commit();
              if (e.key === "Escape") setEdit(false);
            }}
            onBlur={() => setEdit(false)}
            className="w-16 rounded border border-slate-500 bg-slate-900 px-1 py-px font-mono text-xs text-ink-1"
          />
          <button
            onClick={() => void commit()}
            className="rounded px-1.5 py-1 text-xs text-cyan-300 hover:bg-slate-800"
          >
            应用
          </button>
          <button
            onClick={() => setEdit(false)}
            className="rounded px-1.5 py-1 text-xs text-ink-4 hover:text-ink-2"
          >
            取消
          </button>
        </>
      ) : (
        <span
          role="button"
          tabIndex={0}
          className="group/port inline-flex cursor-pointer items-center gap-1 font-mono underline decoration-dotted decoration-transparent underline-offset-2 hover:decoration-current"
          title="单击修改统一转发端口"
          onClick={() => {
            setVal("");
            setErr(null);
            setEdit(true);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === " ") {
              e.preventDefault();
              setVal("");
              setErr(null);
              setEdit(true);
            }
          }}
        >
          {listen}
          <IconPencil
            width={10}
            height={10}
            className="text-ink-4 opacity-0 transition-opacity group-hover/port:opacity-100 group-focus-within/port:opacity-100"
          />
        </span>
      )}
      {err && (
        <span className="absolute left-1/2 top-full z-40 mt-1 -translate-x-1/2 whitespace-nowrap rounded border border-[var(--sem-danger-text)] bg-slate-900 px-2 py-0.5 text-xs text-[var(--sem-danger-text)]">
          {err}
        </span>
      )}
      {confirmPort !== null && (
        <ConfirmDialog
          open
          title={`更换转发端口 :${curPort} → :${confirmPort}？`}
          onClose={() => setConfirmPort(null)}
          onConfirm={() => {
            setBusy(true);
            apply.mutate(confirmPort);
          }}
          confirmLabel="确认更换"
          danger
        >
          网关将<b>短暂重启</b>重新监听，<Risk>进行中的请求会中断</Risk>；
          所有已接入的下游客户端需同步改用新端口。
        </ConfirmDialog>
      )}
    </span>
  );
}
