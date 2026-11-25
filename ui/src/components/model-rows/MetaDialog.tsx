// 模型参数声明弹窗（计划 §6.1 自 ModelRows.tsx 拆出）：ctx/output 与 reasoning/VLM 三态手填。
// 原先是贴在「ctx」按钮下方的就地浮层：字段有 8 个（含单价四维），浮层窄（230px）且
// 贴着行边缘，长说明被挤成一团，靠下的行还会被展开区裁掉。改成窗口居中弹窗（与
// ConfirmDialog / ModelParamsDialog 同族 Radix Dialog），Esc 与点遮罩关闭由 Radix 管。
import * as Dialog from "@radix-ui/react-dialog";
import { useState } from "react";
import type { ModelMeta } from "../../lib/ipc";
import { t, tf, ts } from "../../lib/i18n";
import { fmtTok } from "./fmt";


/** 声明弹窗：ctx/output 0 = 自动（后端真值→探测→未知）、非 0 = 强制（超上限钳并提示）；
 * reasoning/VLM 三态：自动（跟随探测）/ 强制开 / 强制关。 */
export function MetaDialog({
  meta,
  onSave,
  onCancel,
}: {
  meta?: ModelMeta;
  /** 单价四维：null = 未设（空着）；仅在线后端计价。省略 = 本次不动单价。 */
  onSave: (
    ctx: number,
    output: number,
    reasoning: number,
    vlm: number,
    price?: { input: number | null; output: number | null; cache_read: number | null; cache_creation: number | null },
  ) => void;
  onCancel: () => void;
}) {
  const [ctx, setCtx] = useState(String(meta?.manual_ctx ?? 0));
  const [output, setOutput] = useState(String(meta?.manual_output ?? 0));
  const [reasoning, setReasoning] = useState(meta?.manual_reasoning ?? 0);
  const [vlm, setVlm] = useState(meta?.manual_vlm ?? 0);
  const [pIn, setPIn] = useState(meta?.price ? String(meta.price.input) : "");
  const [pOut, setPOut] = useState(meta?.price ? String(meta.price.output) : "");
  const [pCr, setPCr] = useState(meta?.price ? String(meta.price.cache_read) : "");
  const [pCc, setPCc] = useState(meta?.price ? String(meta.price.cache_creation) : "");
  const [err, setErr] = useState("");

  const num = (s: string) => {
    const n = Math.floor(Number(s));
    return Number.isFinite(n) && n > 0 ? n : 0;
  };
  const submit = () => {
    // 单价：全空 = 清除（未设 = 不算）；任一非空 = 设价，空维补 0。
    const pnum = (s: string): number | null => {
      const v = s.trim();
      if (!v) return null;
      const n = Number(v);
      return Number.isFinite(n) && n >= 0 ? n : NaN;
    };
    const [a, b, cr, cc] = [pIn, pOut, pCr, pCc].map(pnum);
    if ([a, b, cr, cc].some((v) => Number.isNaN(v))) {
      setErr(t("md.priceErr"));
      return;
    }
    setErr("");
    const allNull = [a, b, cr, cc].every((v) => v === null);
    onSave(
      num(ctx),
      num(output),
      reasoning,
      vlm,
      allNull
        ? { input: null, output: null, cache_read: null, cache_creation: null }
        : { input: a ?? 0, output: b ?? 0, cache_read: cr ?? 0, cache_creation: cc ?? 0 },
    );
  };

  const tri = (label: string, val: number, set: (v: number) => void, eff: boolean | null) => (
    <label className="flex items-center justify-between gap-2">
      <span className="text-sm text-ink-4">
        {label}
        {eff != null && (
          <span className="ml-1 text-ink-4">{t("md.now")}{eff ? t("md.on") : t("md.off")}</span>
        )}
      </span>
      <select
        value={val}
        onChange={(e) => set(Number(e.target.value))}
        className="rounded border border-slate-600 bg-slate-950 px-1 py-0.5 text-sm text-ink-1 focus-visible:border-cyan-500/60"
      >
        <option value={0}>{t("md.auto")}</option>
        <option value={1}>{t("md.forceOn")}</option>
        <option value={2}>{t("md.forceOff")}</option>
      </select>
    </label>
  );

  const field = (label: string, val: string, set: (v: string) => void, cap: number | null) => (
    <label className="block">
      <span className="mb-0.5 block text-sm text-ink-4">
        {label}
        {ts("md.fieldWrap", {
          auto: t("md.zeroAuto"),
          cap: cap != null ? ts("md.capSuffix", { n: fmtTok(cap) }) : "",
        })}
      </span>
      <input
        inputMode="numeric"
        value={val}
        onChange={(e) => set(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") submit();
          if (e.key === "Escape") onCancel();
        }}
        className="w-full rounded border border-slate-600 bg-slate-950 px-1.5 py-1 font-mono text-xs text-ink-1 focus-visible:border-cyan-500/60"
      />
    </label>
  );

  return (
    <Dialog.Root open onOpenChange={(o) => !o && onCancel()}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-[90] bg-black/60" />
        {/* 高度用百分比不用 vh：zoom 挂在 documentElement 上，vh 会被倍率乘高，
            zoom 到 1.33 时弹窗比窗口还高，标题与按钮被顶出去（ui-budget 门禁）。 */}
        <Dialog.Content className="fixed left-1/2 top-1/2 z-[90] w-[420px] max-w-[92vw] max-h-[86%] overflow-auto -translate-x-1/2 -translate-y-1/2 rounded-xl border border-slate-700 bg-slate-900 p-4 shadow-2xl">
          <Dialog.Title className="text-base font-semibold">{t("md.title")}</Dialog.Title>
      <div className="mt-3 mb-1.5 text-sm leading-relaxed text-ink-3">
        {tf("md.curValid", {
          ctx: <b className="text-ink-2">{fmtTok(meta?.ctx)}</b>,
          out: <b className="text-ink-2">{fmtTok(meta?.output)}</b>,
          rs: (
            <b className="text-ink-2">{meta?.reasoning == null ? "—" : meta.reasoning ? t("md.on") : t("md.off")}</b>
          ),
          vlm: <b className="text-ink-2">{meta?.vlm == null ? "—" : meta.vlm ? t("md.on") : t("md.off")}</b>,
        })}
        <br />
        <span className="text-ink-4">{t("md.autoNote")}</span>
      </div>
      <div className="flex flex-col gap-1.5">
        {field(t("md.ctx"), ctx, setCtx, meta?.cap_ctx ?? null)}
        {field(t("md.output"), output, setOutput, meta?.cap_output ?? null)}
        {tri(t("md.reasoning"), reasoning, setReasoning, meta?.reasoning ?? null)}
        {tri(t("md.vlm"), vlm, setVlm, meta?.vlm ?? null)}
      </div>
      <div className="mt-2 border-t border-slate-800 pt-1.5">
        <div className="mb-1 text-sm text-ink-4">{t("md.priceNote")}</div>
        <div className="grid grid-cols-2 gap-1">
          {([
            [t("md.pIn"), pIn, setPIn],
            [t("md.pOut"), pOut, setPOut],
            [t("md.pCr"), pCr, setPCr],
            [t("md.pCc"), pCc, setPCc],
          ] as const).map(([label, val, set]) => (
            <label key={label} className="flex items-center gap-1">
              <span className="w-9 shrink-0 text-sm text-ink-4">{label}</span>
              <input
                inputMode="decimal"
                value={val}
                placeholder="—"
                onChange={(e) => set(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") submit();
                  if (e.key === "Escape") onCancel();
                }}
                className="w-full rounded border border-slate-600 bg-slate-950 px-1.5 py-0.5 font-mono text-xs text-ink-1 focus-visible:border-cyan-500/60"
              />
            </label>
          ))}
        </div>
      </div>
      {err && <div className="mt-1 text-sm text-[var(--sem-danger-text)]">{err}</div>}
      <div className="mt-2 flex items-center justify-between gap-2 border-t border-slate-800 pt-1.5">
        <button
          onClick={() => onSave(0, 0, 0, 0)}
          className="text-sm text-ink-4 hover:text-ink-2"
          title={t("md.resetTip")}
        >
          {t("md.reset")}
        </button>
        <span className="flex gap-1.5">
          <button onClick={onCancel} className="rounded px-2 py-0.5 text-sm text-ink-3 hover:bg-slate-800">
            {t("dlg.cancel")}
          </button>
          <button
            onClick={submit}
            className="rounded border border-cyan-500/50 bg-cyan-500/15 px-2 py-0.5 text-sm font-medium text-cyan-200 transition-colors hover:bg-cyan-500/25"
          >
            {t("dlg.save")}
          </button>
        </span>
      </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
