// 模型选择器：可手填的行列表 + 「扫描模型」勾选面板。
// 设计取向（对齐 ~/swriter 的提供商设置，两处有意不同）：
//   1) 扫描结果不整表覆盖，而是弹勾选面板让用户「逐一选择是否加入」，手填内容不会被冲掉；
//   2) 只填模型 id，不搞显示名——本仓库 aliases 的语义是「客户端叫法 → 上游真实 id」，
//      拿它当显示名会变垃圾场（友好名请在「模型与参数」页正经配别名）。
import { useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { Button, inputLgCls } from "./ui";
import { IconClose, IconPlus } from "./icons";
import { QueryError } from "./QueryError";
import { ipc } from "../lib/ipc";
import type { ScannedModel } from "../lib/ipc";
import { suggestWorkKind } from "../lib/workkind";
import { t, ts } from "../lib/i18n";

interface Candidate extends ScannedModel {
  display: string | null;
}

interface Discovery {
  source: string;
  url: string;
  models: Candidate[];
}

export interface ProbeTarget {
  base_url: string;
  protocol: string;
  api_key?: string;
}

/** 一次扫描的真值快照：保存时随表单带走（换过连接信息就作废）。 */
export interface ProbeSnapshot {
  /** 扫描那一刻的连接信息。 */
  target: ProbeTarget;
  models: ScannedModel[];
}

/** 连接信息指纹：保存时比对「快照来源」与「当前表单」，对不上就不落真值。 */
export const probeTargetKey = (t: ProbeTarget) =>
  JSON.stringify([t.base_url, t.protocol, t.api_key ?? ""]);

/** 上游真值的紧凑展示（131072 → 128K）。 */
const tok = (n: number) => (n >= 1024 ? `${Math.round(n / 1024)}K` : `${n}`);

export function ModelPicker({
  models,
  onChange,
  target,
  onProbe,
}: {
  models: string[];
  onChange: (next: string[]) => void;
  /** 扫描所需的连接信息，点按钮那一刻才读，避免用户边打字边打网络。 */
  target: () => ProbeTarget;
  /** 扫描成功时回传真值快照；保存端负责比对指纹，对不上就丢弃。 */
  onProbe?: (snap: ProbeSnapshot) => void;
}) {
  const [picked, setPicked] = useState<Record<string, boolean>>({});
  const field = inputLgCls;

  const scan = useMutation({
    mutationFn: () => ipc<Discovery>("probe_models", { input: target() }),
    onSuccess: (d) => {
      // 默认勾上「还没加的」，已存在的默认不勾（避免重复添加）；
      // 媒体工种默认不勾（勾选是用户的主过滤，媒体要显式选入，11d）。
      const next: Record<string, boolean> = {};
      for (const m of d.models)
        next[m.id] = !models.includes(m.id) && suggestWorkKind(m.id) === null;
      setPicked(next);
      onProbe?.({ target: target(), models: d.models });
    },
  });
  const found = scan.data;

  const chosen = found ? found.models.filter((m) => picked[m.id]).map((m) => m.id) : [];

  const setRow = (i: number, v: string) =>
    onChange(models.map((m, j) => (j === i ? v : m)));
  const dropRow = (i: number) => onChange(models.filter((_, j) => j !== i));

  const addChosen = () => {
    const merged = [...models];
    for (const id of chosen) if (!merged.includes(id)) merged.push(id);
    onChange(merged);
    setPicked({});
    scan.reset();
  };

  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between">
        <span className="text-sm text-ink-3">{ts("mp.modelsN", { n: models.filter(Boolean).length })}</span>
        <Button
          onClick={() => scan.mutate()}
          disabled={scan.isPending || !target().base_url.trim()}
          title={target().base_url.trim() ? t("mp.scanTip") : t("mp.fillUrlTip")}
        >
          {scan.isPending ? t("common.scanning") : t("mp.scan")}
        </Button>
      </div>

      <div className="space-y-1.5">
        {models.map((m, i) => (
          <div key={i} className="flex items-center gap-1.5">
            <input
              className={`${field} font-mono text-xs`}
              placeholder={t("mp.idPh")}
              value={m}
              onChange={(e) => setRow(i, e.target.value)}
            />
            <button
              onClick={() => dropRow(i)}
              title={t("mp.removeTip")}
              aria-label={t("mp.removeAria")}
              className="grid h-7 w-7 shrink-0 place-items-center rounded-md text-ink-4 hover:bg-slate-800 hover:text-[var(--sem-danger-text)]"
            >
              <IconClose width={13} height={13} />
            </button>
          </div>
        ))}
        <button
          onClick={() => onChange([...models, ""])}
          className="inline-flex items-center gap-1 text-sm text-cyan-300 hover:underline"
        >
          <IconPlus width={12} height={12} />
          {t("mp.add")}
        </button>
      </div>

      {scan.isError && (
        <QueryError inline title={t("remote.scanFail")} error={scan.error} onRetry={() => scan.mutate()} />
      )}

      {found && (
        <div className="rounded-lg border border-slate-700 bg-slate-800/40 p-2.5">
          <div className="flex items-center justify-between text-sm">
            <span className="text-ink-2">
              {ts("mp.foundN", { n: found.models.length })}
              <span className="ml-1.5 rounded bg-slate-700/60 px-1.5 py-0.5 text-2xs text-ink-2">
                {found.source}
              </span>
            </span>
            <div className="flex gap-2 text-cyan-300">
              <button
                onClick={() =>
                  setPicked(Object.fromEntries(found.models.map((m) => [m.id, true])))
                }
              >
                {t("mp.all")}
              </button>
              <button onClick={() => setPicked({})}>{t("mp.none")}</button>
              <button
                onClick={() =>
                  setPicked(
                    Object.fromEntries(found.models.map((m) => [m.id, !models.includes(m.id)])),
                  )
                }
              >
                {t("mp.onlyNew")}
              </button>
            </div>
          </div>

          <ul className="mt-2 max-h-52 space-y-1 overflow-auto pr-1">
            {found.models.map((m) => {
              const had = models.includes(m.id);
              const wk = suggestWorkKind(m.id);
              return (
                <li key={m.id} className="flex items-center gap-2 text-sm">
                  <input
                    type="checkbox"
                    checked={!!picked[m.id]}
                    onChange={(e) => setPicked({ ...picked, [m.id]: e.target.checked })}
                  />
                  <span className={`font-mono ${had ? "text-ink-4" : "text-ink-2"}`}>
                    {m.id}
                  </span>
                  {m.display && <span className="truncate text-ink-4" title={m.display}>{m.display}</span>}
                  {wk && (
                    <span
                      className="shrink-0 rounded bg-slate-700/60 px-1 text-2xs text-ink-3"
                      title={t("picker.kindTip")}
                    >
                      {t(`wk.${wk}`)}
                    </span>
                  )}
                  {had && <span className="text-sm text-ink-4">{t("mp.exists")}</span>}
                  {(m.ctx != null || m.output != null) && (
                    <span
                      className="ml-auto shrink-0 text-sm text-ink-4"
                      title={t("mp.ctxTip")}
                    >
                      {m.ctx != null ? tok(m.ctx) : "—"}
                      {m.output != null && `/${tok(m.output)}`}
                    </span>
                  )}
                </li>
              );
            })}
          </ul>

          <div className="mt-2 flex items-center justify-between">
            <span className="text-sm text-ink-4">{found.url}</span>
            <Button onClick={addChosen} disabled={chosen.length === 0}>
              {ts("mp.addChosen", { n: chosen.length })}
            </Button>
          </div>
        </div>
      )}
    </div>
  );
}
