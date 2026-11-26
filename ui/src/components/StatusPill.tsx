// 顶栏代理状态胶囊（从 App.tsx 原样搬出）：连接中 / 已暂停 / 运行中三态，
// 含统一转发端口的原位编辑（校验链 + 占用探测 + 二次确认，决策 D1、任务 3.2）。
import { useState } from "react";
import type { KeyboardEvent as ReactKeyboardEvent } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useLive } from "../lib/live";
import { useAcceptToggle } from "../lib/accept";
import { StatusDot } from "./ui";
import { useToast } from "./Toast";
import { ConfirmDialog, Risk } from "./ConfirmDialog";
import { IconPencil } from "./icons";
import { t, tf, ts } from "../lib/i18n";
import { useTimedNote } from "../lib/timedNote";
import { ipc, type ShellStatus } from "../lib/ipc";

function fmtTokens(n: number) {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}

export function StatusPill({ status, error }: { status?: ShellStatus; error: boolean }) {
  // 顶栏那颗点也跟流量走：有请求在飞才呼吸，没请求就老实亮绿。
  const live = useLive(1500);
  // 双击胶囊切换接单（用户 2026-11-18：不要单独摆按钮）。键盘同样能切（Enter / 空格），
  // 不能只给鼠标；读不到状态时不绑处理器，避免双击一个开关不动还让人以为坏了。
  const accept = useAcceptToggle();
  const acceptCtl =
    accept.failed || !accept.ready
      ? {}
      : {
          role: "button" as const,
          tabIndex: 0,
          "data-tip": `${accept.tip} · ${t("top.pillDbl")}`,
          "aria-label": `${t("top.acceptAria")}（${t("top.pillDbl")}）`,
          onDoubleClick: accept.toggle,
          onKeyDown: (e: ReactKeyboardEvent) => {
            if (e.key === "Enter" || e.key === " ") {
              e.preventDefault();
              accept.toggle();
            }
          },
        };
  const dbl = accept.failed || !accept.ready ? "" : " cursor-pointer";
  if (error || !status) {
    return (
      <span className="flex h-8 items-center gap-2 rounded-full border border-slate-700 bg-slate-800/60 px-3 text-sm text-ink-3">
        {t("top.connecting")}
      </span>
    );
  }
  if (status.paused) {
    return (
      <span
        {...acceptCtl}
        className={`tip flex h-8 items-center gap-2 rounded-full border border-amber-500/40 bg-amber-500/10 px-3 text-sm text-[var(--sem-busy-text)]${dbl}`}
      >
        <span className="h-1.5 w-1.5 rounded-full bg-amber-400" />
        {t("top.paused")} · {status.listen}
        {/* 窄窗口（<lg）先丢「今日 tokens」这段次要信息，保住端口与状态本体（UI 审查 P2-9） */}
        <span className="tip hidden lg:inline" data-tip={t("top.todayTokens")}>
          {" "}
          {ts("top.todayN", { n: fmtTokens(status.today_tokens) })}
        </span>
      </span>
    );
  }
  return (
    <span
      {...acceptCtl}
      className={`tip flex h-8 items-center gap-2 rounded-full border border-emerald-500/30 bg-emerald-500/10 px-3 text-sm text-[var(--sem-ok-text)]${dbl}`}
    >
      <StatusDot busy={live.total > 0} />
      {t("top.running")} · <ListenAddr listen={status.listen} />
      <span className="tip hidden lg:inline" data-tip={t("top.todayTokens")}>
        {" "}
        {ts("top.todayN", { n: fmtTokens(status.today_tokens) })}
      </span>
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
  // 就地报错气泡：文案与自清定时器都交给 useTimedNote（连点不会让旧定时器提前抹掉新提示）。
  const { note: err, show: flashErr, clear: clearErr } = useTimedNote(3000);
  const [confirmPort, setConfirmPort] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);

  const apply = useMutation({
    mutationFn: (p: number) => ipc<string>("set_listen_port", { port: p }),
    onSuccess: (listenNew) => {
      // 乐观更新胶囊（status 轮询 5s 才回，不等它）；后台失败会在下次轮询里纠偏。
      qc.setQueryData(["status"], (old: ShellStatus | undefined) =>
        old ? { ...old, listen: listenNew } : old,
      );
      qc.invalidateQueries({ queryKey: ["status"] });
    },
    onError: (e) => toast.err(ts("top.portSaveFail", { e: String(e) })),
    onSettled: () => setBusy(false),
  });

  /** 校验 → 占用探测 → 二次确认。出错时留在编辑态，让用户就地改而不是重敲。 */
  const commit = async () => {
    const v = val.trim();
    if (!v) return setEdit(false); // 空值 = 放弃
    const p = Number(v);
    if (!/^\d+$/.test(v) || p < 1 || p > 65535) return flashErr(t("top.portInvalid"));
    if (p === curPort) return setEdit(false); // 没动就不打扰
    try {
      const free = await ipc<boolean>("check_port", { port: p });
      if (!free) return flashErr(ts("top.portBusy", { p }));
    } catch (e) {
      return flashErr(ts("top.portProbeFail", { e: String(e) }));
    }
    setEdit(false);
    setConfirmPort(p);
  };

  if (busy) {
    return <span className="text-ink-3">{t("top.restarting")}</span>;
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
            aria-label={t("top.portAria")}
            onChange={(e) => {
              setVal(e.target.value);
              clearErr();
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
            className="rounded px-1.5 py-1 text-sm text-cyan-300 hover:bg-slate-800"
          >
            {t("top.apply")}
          </button>
          <button
            onClick={() => setEdit(false)}
            className="rounded px-1.5 py-1 text-sm text-ink-4 hover:text-ink-2"
          >
            {t("dlg.cancel")}
          </button>
        </>
      ) : (
        <span
          role="button"
          tabIndex={0}
          className="group/port inline-flex cursor-pointer items-center gap-1 font-mono underline decoration-dotted decoration-transparent underline-offset-2 hover:decoration-current"
          title={t("top.portTip")}
          // 端口原位编辑的入口必须留着（用户 2026-11-18）：胶囊整体挂了「双击切换接单」，
          // 这里把 click / dblclick 都拦下，点端口只进编辑，不会顺手把接单也切了。
          onClick={(e) => {
            e.stopPropagation();
            setVal("");
            clearErr();
            setEdit(true);
          }}
          onDoubleClick={(e) => e.stopPropagation()}
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === " ") {
              e.preventDefault();
              setVal("");
              clearErr();
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
        <span className="absolute left-1/2 top-full z-40 mt-1 -translate-x-1/2 whitespace-nowrap rounded border border-[var(--sem-danger-text)] bg-slate-900 px-2 py-0.5 text-sm text-[var(--sem-danger-text)]">
          {err}
        </span>
      )}
      {confirmPort !== null && (
        <ConfirmDialog
          open
          title={ts("top.portChangeTitle", { from: curPort, to: confirmPort })}
          onClose={() => setConfirmPort(null)}
          onConfirm={() => {
            setBusy(true);
            apply.mutate(confirmPort);
          }}
          confirmLabel={t("top.portChangeConfirm")}
          danger
        >
          {tf("top.portChangeBody", {
            b: <b>{t("top.portBounce")}</b>,
            risk: <Risk>{t("top.portDropRisk")}</Risk>,
          })}
        </ConfirmDialog>
      )}
    </span>
  );
}
