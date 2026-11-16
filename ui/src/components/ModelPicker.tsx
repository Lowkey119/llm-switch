// 模型选择器：可手填的行列表 + 「扫描模型」勾选面板。
// 设计取向（对齐 ~/swriter 的提供商设置，两处有意不同）：
//   1) 扫描结果不整表覆盖，而是弹勾选面板让用户「逐一选择是否加入」，手填内容不会被冲掉；
//   2) 只填模型 id，不搞显示名——本仓库 aliases 的语义是「客户端叫法 → 上游真实 id」，
//      拿它当显示名会变垃圾场（友好名请在「模型与参数」页正经配别名）。
import { useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { Button, inputLgCls } from "./ui";
import { QueryError } from "./QueryError";
import { ipc } from "../lib/ipc";
import type { ScannedModel } from "../lib/ipc";

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
      // 默认勾上「还没加的」，已存在的默认不勾（避免重复添加）。
      const next: Record<string, boolean> = {};
      for (const m of d.models) next[m.id] = !models.includes(m.id);
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
        <span className="text-xs text-ink-3">模型（{models.filter(Boolean).length}）</span>
        <Button
          onClick={() => scan.mutate()}
          disabled={scan.isPending || !target().base_url.trim()}
          title={target().base_url.trim() ? "问后端要模型列表" : "先填 Base URL"}
        >
          {scan.isPending ? "扫描中…" : "扫描模型"}
        </Button>
      </div>

      <div className="space-y-1.5">
        {models.map((m, i) => (
          <div key={i} className="flex items-center gap-1.5">
            <input
              className={`${field} font-mono text-xs`}
              placeholder="模型 id"
              value={m}
              onChange={(e) => setRow(i, e.target.value)}
            />
            <button
              onClick={() => dropRow(i)}
              title="移除这一行"
              aria-label="移除模型"
              className="grid h-7 w-7 shrink-0 place-items-center rounded-md text-ink-4 hover:bg-slate-800 hover:text-[var(--sem-danger-text)]"
            >
              ✕
            </button>
          </div>
        ))}
        <button
          onClick={() => onChange([...models, ""])}
          className="text-xs text-cyan-300 hover:underline"
        >
          + 添加模型
        </button>
      </div>

      {scan.isError && (
        <QueryError inline title="扫描失败" error={scan.error} onRetry={() => scan.mutate()} />
      )}

      {found && (
        <div className="rounded-lg border border-slate-700 bg-slate-800/40 p-2.5">
          <div className="flex items-center justify-between text-xs">
            <span className="text-ink-2">
              发现 {found.models.length} 个模型
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
                全选
              </button>
              <button onClick={() => setPicked({})}>取消全选</button>
              <button
                onClick={() =>
                  setPicked(
                    Object.fromEntries(found.models.map((m) => [m.id, !models.includes(m.id)])),
                  )
                }
              >
                只选新增
              </button>
            </div>
          </div>

          <ul className="mt-2 max-h-52 space-y-1 overflow-auto pr-1">
            {found.models.map((m) => {
              const had = models.includes(m.id);
              return (
                <li key={m.id} className="flex items-center gap-2 text-xs">
                  <input
                    type="checkbox"
                    checked={!!picked[m.id]}
                    onChange={(e) => setPicked({ ...picked, [m.id]: e.target.checked })}
                  />
                  <span className={`font-mono ${had ? "text-ink-4" : "text-ink-2"}`}>
                    {m.id}
                  </span>
                  {m.display && <span className="truncate text-ink-4" title={m.display}>{m.display}</span>}
                  {had && <span className="text-xs text-ink-4">已存在</span>}
                  {(m.ctx != null || m.output != null) && (
                    <span
                      className="ml-auto shrink-0 text-xs text-ink-4"
                      title="上游给的上下文/输出上限（保存时一并落库；没给就不显示）"
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
            <span className="text-xs text-ink-4">{found.url}</span>
            <Button onClick={addChosen} disabled={chosen.length === 0}>
              添加所选（{chosen.length}）
            </Button>
          </div>
        </div>
      )}
    </div>
  );
}
