// 模型行两行制（UI 重设计定稿，docs/ui_pages/01-后端页.md）：
// 第 1 行 = 数据流点 + 映射名 + pf/dec 速率 + ctx(实际/支持) + VLM ｜ 暴露开关 + 参数 + 删除；
// 第 2 行 = 代理分配灯组：点灯就地弹该端槽位浮层完成分配（与矩阵同源交互），
// 未接管的端退回跳代理页矩阵（先接管才能写）。
// 速率口径：空闲显历史均值、在飞切实时（后端 model_metrics 已择源）；无数据一律 `—`。
// 单模型行的三行制细节在 model-rows/ModelItem.tsx；就地分配浮层在 model-rows/AllocPopover.tsx。
import { useState, type MouseEvent } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useToast } from "./Toast";
import {
  ipc,
  type BackendRow,
  type MatrixCell,
  type MatrixClient,
  type ModelMeta,
  type ModelMetaSetResult,
  type ModelMetric,
} from "../lib/ipc";
import { detectKind, KIND_LABEL, KIND_ORDER } from "../lib/modelkind";
import { resolveWorkKind, WORK_ORDER, type WorkKind } from "../lib/workkind";
import { t, ts } from "../lib/i18n";
import type { ParamRow } from "../lib/params";
import { ModelItem } from "./model-rows/ModelItem";
import { AllocPopover, type AllocPop } from "./model-rows/AllocPopover";
import { gotoMatrix } from "../lib/nav";

interface Props {
  b: BackendRow;
  paramRows: ParamRow[];
  /** 模型名 → 指标（model_metrics 轮询结果，缺项 = 还没数据）。 */
  metrics: Record<string, ModelMetric>;
  /** 模型名 → 矩阵格子（app → cell），来自 matrix_get。 */
  cells: Record<string, Record<string, MatrixCell>>;
  /** 矩阵列（app → 端定义），来自 matrix_get：点灯就地分配要查槽位与接管态。 */
  clients: MatrixClient[];
  /** 占用表（app → slot → 模型）：浮层里标「将替换」，与矩阵页同源算法。 */
  occ: Record<string, Record<string, string>>;
  onOpenParams: (model: string) => void;
  onChanged: () => void;
}

export function ModelRows({
  b,
  paramRows,
  metrics,
  cells,
  clients,
  occ,
  onOpenParams,
  onChanged,
}: Props) {
  const toast = useToast();
  const [confirming, setConfirming] = useState<string | null>(null);
  const [metaFor, setMetaFor] = useState<string | null>(null); // 开着元数据声明浮层的模型名
  // 就地分配浮层：哪个模型的哪个端、弹在点击点（与矩阵格子同一套交互）。
  const [pop, setPop] = useState<AllocPop | null>(null);

  // 元数据真值（全量一次取，行内就地声明的数据源）。
  const metaQ = useQuery({
    queryKey: ["model-meta"],
    queryFn: () => ipc<ModelMeta[]>("model_meta_get"),
    refetchOnWindowFocus: false,
  });
  const qc = useQueryClient();
  const metaOf = (m: string) => metaQ.data?.find((x) => x.model === m);

  const run = async (msg: string, fn: () => Promise<unknown>) => {
    try {
      await fn();
      onChanged();
      toast.ok(msg);
    } catch (e) {
      toast.err(ts("ag.actFail", { e: String(e) }));
    }
  };

  /**
   * 分配灯点击 = 就地分配（与矩阵格子同源语义：同一模型可同时占各端的槽，
   * 同端同槽被占才顶替）。分三类：槽位制端弹槽位浮层；gemini 单槽直设默认；
   * 清单制端（copilot）直切清单成员。未接管的端就地分配——写下去即自动接管
   * （用户 2026-11-17），只有查不到这个端时才退回跳矩阵。
   */
  const chipClick = (app: string, model: string, cell: MatrixCell | undefined, e: MouseEvent) => {
    const client = clients.find((c) => c.app === app);
    if (!client) {
      gotoMatrix();
      return;
    }
    if (client.slots.length > 0) {
      setPop({ app, model, x: e.clientX, y: e.clientY, kind: "slots" });
      return;
    }
    if (client.capacity === 1 && app === "gemini") {
      if (cell?.is_default) toast.info(t("mx.geminiOne"));
      else
        void run(ts("ap.setAsModel", { app: "gemini" }), () =>
          ipc("matrix_cell", { app, model, op: "default", slot: null }),
        );
      return;
    }
    // 清单制端（opencode / DevEco）：点这盏灯的心智是「让这个代理用它」，不是
    // 「只加进可见清单」。原先这里发 op=add，只把模型写进 provider.models，
    // 顶层 model 一个字都不动——用户看到的就是「点了没反应，还得去代理页换模型」。
    // 现在：没亮 → 直接设为该代理的模型（后端顺带补进清单）；亮着但不是当前模型 →
    // 就地给「设为当前 / 移出清单」两个动作；已经是当前模型 → 说清楚，不静默。
    if (cell?.is_default) {
      // 已经是当前模型也要能就地改：用户在这儿才发现分错了，只弹一句 toast 就等于
      // 「只能分一次」。浮层里给「设为当前 / 移出清单」，取消与改分配都在原位完成。
      setPop({ app, model, x: e.clientX, y: e.clientY, kind: "catalog" });
      return;
    }
    if (cell?.in_list) {
      setPop({ app, model, x: e.clientX, y: e.clientY, kind: "catalog" });
      return;
    }
    void run(ts("ap.setAsModel", { app }), () =>
      ipc("matrix_cell", { app, model, op: "default", slot: null }),
    );
  };

  /** 就地声明 ctx/output/reasoning/VLM + 手工单价：ctx/output 0 = 自动、非 0 = 强制（超上限后端钳）；
   * 三态 0 自动/1 强制开/2 强制关；单价四维全 null = 清除（未设 = 不算，仅在线后端计价）。
   * 保存后后端会自动重写受管端的接管配置。 */
  const saveMeta = async (
    model: string,
    ctx: number,
    output: number,
    reasoning: number,
    vlm: number,
    price?: {
      input: number | null;
      output: number | null;
      cache_read: number | null;
      cache_creation: number | null;
    },
  ) => {
    try {
      const res = await ipc<ModelMetaSetResult>("model_meta_set", {
        model,
        ctx,
        output,
        reasoning,
        vlm,
      });
      if (price) {
        await ipc("model_price_set", {
          model,
          input: price.input,
          output: price.output,
          // Tauri v2 命令参数一律按 camelCase 收。
          cacheRead: price.cache_read,
          cacheCreation: price.cache_creation,
        });
      }
      setMetaFor(null);
      await qc.invalidateQueries({ queryKey: ["model-meta"] });
      await qc.invalidateQueries({ queryKey: ["metrics"] });
      await qc.invalidateQueries({ queryKey: ["usage-cost"] });
      toast.ok(res.message);
      onChanged();
    } catch (e) {
      toast.err(ts("row.declareFail", { e: String(e) }));
    }
  };

  // 主分组轴 = 工种类（chat/image_gen/tts/asr/video_gen，与后端路由同一口径，
  // 见 lib/workkind）；聊天组内再按架构种类（LLM/MoE/VLM/向量化/重排…）打子标签
  // ——旧的工种类主轴降级为聊天组的子标签（媒体端点透传代理 11d）。
  // 单组且是聊天时不打组标题（少打扰，旧规则沿用）。
  const flagOf = (m: string) => b.model_flags?.find((f) => f.public_name === m);
  const workGroups = WORK_ORDER.map((wk) => ({
    wk,
    models: b.models.filter((m) => resolveWorkKind(flagOf(m), m) === wk),
  })).filter((g) => g.models.length > 0);
  const showWorkTitles = workGroups.length > 1 || workGroups[0]?.wk !== "chat";

  // 聊天组内的架构子分组：种类判定 = 名称启发式 + model_meta.vlm 探测事实；
  // 只有 llm 一种时不打子标题（旧规则沿用）。
  const chatSubGroups = (ms: string[]) => {
    const gs = KIND_ORDER.map((kind) => ({
      kind,
      models: ms.filter((m) => detectKind(m, metaOf(m)?.vlm) === kind),
    })).filter((g) => g.models.length > 0);
    return gs.length > 1 || gs[0]?.kind !== "llm" ? gs : null;
  };

  const renderItem = (m: string, wk: WorkKind) => (
    <ModelItem
      key={m}
      b={b}
      m={m}
      work={wk}
      paramRows={paramRows}
      metrics={metrics}
      cells={cells}
      metaQ={metaQ}
      metaFor={metaFor}
      setMetaFor={setMetaFor}
      confirming={confirming}
      setConfirming={setConfirming}
      run={run}
      saveMeta={saveMeta}
      chipClick={chipClick}
      onOpenParams={onOpenParams}
    />
  );

  return (
    <div>
      {workGroups.map(({ wk, models: ms }) => {
        const subs = wk === "chat" ? chatSubGroups(ms) : null;
        return (
          <div key={wk} className={showWorkTitles ? "pt-3 first:pt-0" : ""}>
            {showWorkTitles && (
              <div className="flex items-center gap-1.5 pb-1 text-2xs font-medium uppercase tracking-wide text-ink-4">
                <span>{t(`wk.${wk}`)}</span>
                <span className="text-ink-4">· {ms.length}</span>
              </div>
            )}
            {subs ? (
              subs.map(({ kind, models: sub }) => (
                <div key={kind}>
                  <div className="flex items-center gap-1.5 pb-1 text-2xs text-ink-4">
                    <span>{t(KIND_LABEL[kind])}</span>
                    <span>· {sub.length}</span>
                  </div>
                  <div className="divide-y divide-slate-800">
                    {sub.map((m) => renderItem(m, wk))}
                  </div>
                </div>
              ))
            ) : (
              <div className="divide-y divide-slate-800">{ms.map((m) => renderItem(m, wk))}</div>
            )}
          </div>
        );
      })}

      <AllocPopover
        pop={pop}
        setPop={setPop}
        clients={clients}
        cells={cells}
        occ={occ}
        run={run}
      />
    </div>
  );
}
