// 键交互设置（C4，计划 `2026-11-21-SayAll对标-组合序列与App绑定` §三）：
// 双击窗 / 长按窗 / 按住模式（once·repeatSingle·repeatLong）/ 连发起始与间隔。
//
// 手速是身体差异：同一组默认窗，手快的人按出双击、手慢的人按成长按。这里把窗交给用户，
// 但只给区间（后端同样按边界截），自由数只会让人把判定调到不可理喻。
// 弹层里显式「保存」：五个数逐键即存会在用户打到一半时就把半截值发出去。
import { useEffect, useState } from "react";
import * as Dialog from "@radix-ui/react-dialog";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../../components/ui";
import { useToast } from "../../components/Toast";
import { errText } from "../../components/QueryError";
import { IconClock } from "../../components/icons";
import { ipc } from "../../lib/ipc";
import { t } from "../../lib/i18n";

export interface KeyInteraction {
  timing: {
    double_ms: number;
    long_ms: number;
    repeat_start_ms: number;
    repeat_interval_ms: number;
  };
  hold: "once" | "repeat_single" | "repeat_long";
}

/** 与后端 `KeyTiming` 的区间一致（越界后端也会截，这里只是提前把话说清）。 */
const RANGE = { double_ms: [120, 800], long_ms: [300, 1500], repeat_start_ms: [150, 1000], repeat_interval_ms: [30, 500] };

const FIELDS: { id: keyof KeyInteraction["timing"]; labelKey: string; tipKey: string }[] = [
  { id: "double_ms", labelKey: "remote.timing.double", tipKey: "remote.timing.doubleTip" },
  { id: "long_ms", labelKey: "remote.timing.long", tipKey: "remote.timing.longTip" },
  { id: "repeat_start_ms", labelKey: "remote.timing.repeatStart", tipKey: "remote.timing.repeatStartTip" },
  { id: "repeat_interval_ms", labelKey: "remote.timing.repeatInterval", tipKey: "remote.timing.repeatIntervalTip" },
];

const HOLDS: { id: KeyInteraction["hold"]; labelKey: string; tipKey: string }[] = [
  { id: "once", labelKey: "remote.timing.holdOnce", tipKey: "remote.timing.holdOnceTip" },
  { id: "repeat_single", labelKey: "remote.timing.holdSingle", tipKey: "remote.timing.holdSingleTip" },
  { id: "repeat_long", labelKey: "remote.timing.holdLong", tipKey: "remote.timing.holdLongTip" },
];

export function KeyTimingPanel() {
  const qc = useQueryClient();
  const toast = useToast();
  const [open, setOpen] = useState(false);
  const loaded = useQuery({
    queryKey: ["remote-key-interaction"],
    queryFn: () => ipc<KeyInteraction>("remote_key_interaction_get"),
    enabled: open,
  });
  // 弹层内是草稿：关掉再开回到生效值，改一半不污染设置。
  const [draft, setDraft] = useState<KeyInteraction | null>(null);
  const v = draft ?? loaded.data ?? null;
  useEffect(() => {
    if (!open) setDraft(null);
  }, [open]);
  const dirty = !!draft && JSON.stringify(draft) !== JSON.stringify(loaded.data);

  const save = async () => {
    if (!draft) return;
    try {
      const saved = await ipc<KeyInteraction>("remote_key_interaction_set", { def: draft });
      // 回的是后端截完的真值：显示生效值而不是用户敲的值，免得以为没存进去。
      qc.setQueryData(["remote-key-interaction"], saved);
      setDraft(null);
      toast.ok(t("remote.timing.saved"));
    } catch (e) {
      toast.err(`${t("remote.timing.fail")}：${errText(e)}`);
    }
  };

  // 输入框里先按区间截一遍：后端也会截，但用户该立刻看到生效值而不是等保存回执。
  const patchTiming = (id: keyof KeyInteraction["timing"], raw: string) => {
    if (!v) return;
    const [lo, hi] = RANGE[id];
    const n = Math.min(hi, Math.max(lo, Math.round(Number(raw) || 0)));
    setDraft({ ...v, timing: { ...v.timing, [id]: n } });
  };

  return (
    <>
      <button
        type="button"
        onClick={() => setOpen(true)}
        aria-expanded={open}
        data-tip={t("remote.timing.tip")}
        className="tip -my-0.5 flex items-center gap-1.5 rounded px-1.5 py-0.5 transition-colors hover:text-ink-1"
      >
        <IconClock className="h-3 w-3 shrink-0" />
        {t("remote.timing.title")}
      </button>
      <Dialog.Root open={open} onOpenChange={setOpen}>
        <Dialog.Portal>
          <Dialog.Overlay className="fixed inset-0 z-[90] bg-black/60" />
          <Dialog.Content className="fixed left-1/2 top-1/2 z-[90] w-[520px] max-w-[94%] -translate-x-1/2 -translate-y-1/2 rounded-xl border border-slate-700 bg-slate-900 p-5 shadow-2xl">
            <Dialog.Title className="text-base font-semibold">{t("remote.timing.title")}</Dialog.Title>
            <Dialog.Description className="mt-1 text-sm leading-relaxed text-ink-3">
              {t("remote.timing.desc")}
            </Dialog.Description>
            {loaded.isError ? (
              <p className="mt-4 text-sm text-[var(--sem-danger-text)]">{errText(loaded.error)}</p>
            ) : !v ? (
              <p className="mt-4 text-sm text-ink-4">{t("common.loading")}</p>
            ) : (
              <>
                <div className="mt-4 grid grid-cols-2 gap-3">
                  {FIELDS.map((f) => (
                    <label key={f.id} className="tip flex flex-col gap-1" data-tip={t(f.tipKey)}>
                      <span className="text-sm text-ink-2">{t(f.labelKey)}</span>
                      <span className="flex items-center gap-2">
                        <input
                          type="number"
                          min={RANGE[f.id][0]}
                          max={RANGE[f.id][1]}
                          step={10}
                          className="w-24 rounded-lg border border-slate-700 bg-slate-950 px-2 py-1 text-sm tabular-nums text-ink-1 focus:border-[var(--key-hl-text)]"
                          aria-label={t(f.labelKey)}
                          value={v.timing[f.id]}
                          onChange={(e) => patchTiming(f.id, e.target.value)}
                        />
                        <span className="text-sm text-ink-4">{t("remote.timing.ms")}</span>
                      </span>
                    </label>
                  ))}
                </div>
                <div className="mt-4 flex flex-col gap-2">
                  <span className="text-sm text-ink-2">{t("remote.timing.hold")}</span>
                  <div role="group" aria-label={t("remote.timing.hold")} className="flex flex-wrap items-center gap-1.5 rounded-lg border border-slate-700 bg-slate-950 p-1">
                    {HOLDS.map((h) => {
                      const on = v.hold === h.id;
                      return (
                        <button
                          key={h.id}
                          type="button"
                          aria-pressed={on}
                          data-tip={t(h.tipKey)}
                          onClick={() => setDraft({ ...v, hold: h.id })}
                          className={`tip rounded-md px-2.5 py-1 text-sm transition-colors ${
                            on ? "bg-[var(--key-hl)]/15 text-[var(--key-hl-text)]" : "text-ink-3 hover:bg-slate-800 hover:text-ink-1"
                          }`}
                        >
                          {t(h.labelKey)}
                        </button>
                      );
                    })}
                  </div>
                </div>
                <div className="mt-5 flex items-center justify-end gap-2">
                  <Button variant="ghost" onClick={() => setOpen(false)}>
                    {t("remote.timing.close")}
                  </Button>
                  <Button variant="primary" onClick={() => void save()} disabled={!dirty}>
                    {t("remote.timing.save")}
                  </Button>
                </div>
              </>
            )}
          </Dialog.Content>
        </Dialog.Portal>
      </Dialog.Root>
    </>
  );
}
