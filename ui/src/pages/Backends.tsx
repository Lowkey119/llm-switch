// 后端页（UI 重设计定稿，docs/ui_pages/01-后端页.md）。
// ①区行：闪烁点 + 名称 + url + 掩码 key ｜ 刷新(探活+重扫，二次确认) + 编辑 + 暴露开关
//   + webui + 删除；url / key 单击原位编辑（Enter 保存、Esc 或失焦放弃）。
//   删除有两条路：行尾这颗垃圾桶（显式退场），或摘光最后一个模型时连带删除。
// ②区：ModelRows 两行制（指标 + 代理分配）。顶层 Tab（后端/JSON）维持现状。
import { useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useToast } from "../components/Toast";
import { Tabs } from "../components/Tabs";
import { every, POLL } from "../lib/poll";
import { t, tf, ts } from "../lib/i18n";
import { ParamsCenter } from "../components/ParamsCenter";
import { MigrationNote } from "../components/MigrationNote";
import { QueryError } from "../components/QueryError";
import { useLive } from "../lib/live";
import { BackendRowItem } from "./backends/BackendRowItem";
import { BackendDialog } from "./backends/BackendDialog";
import { JsonTab } from "../components/JsonTab";
import { ConsolePage } from "./Console";
import { ConfirmDialog, Risk } from "../components/ConfirmDialog";
import { isProtectedHalogen } from "../lib/backend";
import type { ParamRow } from "../lib/params";
import { ModelParamsDialog } from "../components/ModelParamsDialog";
import { TextButton, SkeletonRows } from "../components/ui";

import { IconBackend, IconSearch } from "../components/icons";
import { EmptyState } from "../components/States";
import { ClearFiltersButton } from "../components/FilterBar";
import { BackendFilterBar } from "./backends/BackendFilterBar";
import { useBackendFilter } from "./backends/useBackendFilter";
import { buildOccupancy } from "../components/model-rows/alloc";
import {
  ipc,
  type BackendInput,
  type BackendRow,
  type DeleteOutcome,
  type MatrixCell,
  type MatrixView,
  type ModelMeta,
  type ModelMetric,
} from "../lib/ipc";
import { detectKind } from "../lib/modelkind";

export function BackendsPage({
  paramsBackend,
  consoleBackend,
  consolePulse,
}: {
  paramsBackend?: string;
  consoleBackend?: string | null;
  consolePulse?: number;
}) {
  const qc = useQueryClient();
  const live = useLive();
  const [adding, setAdding] = useState(false);
  // 顶层 Tab：后端列表 / 参数中心 / JSON 直编 / 控制台（控制台自旧独立页并入，融合原型回灌）。
  // 参数中心原本是独立页「模型与参数」，而参数本来就是后端/模型的属性（决策 D3）。
  const [tab, setTab] = useState<"backends" | "params" | "json" | "console">("backends");
  // 页签行右侧工具条槽位（用户 2026-11-21）：控制台/参数中心把各自工具条 portal 进来，
  // React 树仍挂在子组件下，状态不用上提。ref 回调进 state，子组件挂载后即可投。
  const [tabSlot, setTabSlot] = useState<HTMLDivElement | null>(null);
  // 「去控制台」入口（后端行/参数中心）→ 同页切到控制台页签；脉冲自增保证重复点同一目标也切得回。
  useEffect(() => {
    if (consolePulse) setTab("console");
  }, [consolePulse]);
  const [editing, setEditing] = useState<BackendRow | null>(null);
  const toast = useToast();
  // key = backend id：列表刷新后展开态跟着稳定的 id 走。
  const [expandedIds, setExpandedIds] = useState<Record<number, boolean>>({});
  // ⟳ 二次确认的目标后端（防丢数据闸门）。
  const [refreshTarget, setRefreshTarget] = useState<BackendRow | null>(null);
  // 删除二次确认的目标后端（破坏性动作，同款闸门）。
  const [deleteTarget, setDeleteTarget] = useState<BackendRow | null>(null);

  const backends = useQuery({
    queryKey: ["backends"],
    queryFn: () => ipc<BackendRow[]>("list_backends"),
  });
  // 已接管代理数（直连确认框/toast 的 N）：与代理页同一 queryKey，缓存共用不多打一次。
  const takeover = useQuery({
    queryKey: ["takeover"],
    queryFn: () => ipc<{ clients: { managed: boolean }[] }>("takeover_status"),
  });
  const managedCount = takeover.data?.clients.filter((c) => c.managed).length ?? 0;
  // 视觉模型计数（徽标上移到行头身份区，用户草图口径）：种类判定与展开区同一口径
  // （名称启发式 + model_meta.vlm 探测事实），queryKey 与 ModelRows 共用不多打一次。
  const modelMeta = useQuery({
    queryKey: ["model-meta"],
    queryFn: () => ipc<ModelMeta[]>("model_meta_get"),
    refetchOnWindowFocus: false,
  });
  const vlmCountOf = (b: BackendRow) =>
    b.models.filter((m) => {
      const meta = modelMeta.data?.find((x) => x.model === m);
      return detectKind(m, meta?.vlm) === "vlm";
    }).length;
  // 参数覆盖：与模型参数弹窗共用同一查询缓存（IPC 返回裸数组）。
  const params = useQuery({
    queryKey: ["params"],
    queryFn: () => ipc<ParamRow[]>("list_param_overrides"),
  });
  const matrix = useQuery({
    queryKey: ["matrix"],
    queryFn: () => ipc<MatrixView>("matrix_get"),
  });
  // ②区指标：有任一行展开才 1s 轮询（折叠不烧钱）。
  const anyExpanded = Object.values(expandedIds).some(Boolean);
  const metrics = useQuery({
    queryKey: ["model-metrics"],
    queryFn: () => ipc<ModelMetric[]>("model_metrics"),
    refetchInterval: () => (anyExpanded && tab === "backends" ? every(POLL.live) : false),
    enabled: tab === "backends",
  });
  // 就地参数编辑弹窗（U-B）：记后端名 + 模型名。
  const [paramTarget, setParamTarget] = useState<{
    backend: string;
    model: string;
  } | null>(null);

  const paramRows = params.data ?? [];
  const metricMap: Record<string, ModelMetric> = {};
  for (const m of metrics.data ?? []) metricMap[m.model] = m;
  const cellMap: Record<string, Record<string, MatrixCell>> = {};
  for (const r of matrix.data?.rows ?? []) cellMap[r.model] = r.cells;
  // 占用表（app → slot → 模型）：模型行点灯就地分配的浮层要标「将替换」，与矩阵页同源算法。
  const occByApp = matrix.data ? buildOccupancy(matrix.data) : {};

  const refresh = () => qc.invalidateQueries({ queryKey: ["backends"] });
  const refreshAll = () => {
    refresh();
    qc.invalidateQueries({ queryKey: ["params"] });
    qc.invalidateQueries({ queryKey: ["matrix"] });
    qc.invalidateQueries({ queryKey: ["model-metrics"] });
  };

  const toggleMut = useMutation({
    mutationFn: (v: { name: string; enabled: boolean }) =>
      ipc<{ changed: boolean; also_disabled: string[] }>("set_backend_enabled", {
        name: v.name,
        enabled: v.enabled,
      }),
    onSuccess: (r, v) => {
      // halogen 与 gufo 同端口互斥（内存只够一家）：开一家后端会顺手关另一家，
      // 动了谁必须说清，不然用户以为开关坏了。
      if (r.also_disabled.length) {
        toast.ok(
          ts("bk.toast.mutualDisabled", { name: v.name, others: r.also_disabled.join("、") }),
        );
      }
      refresh();
    },
    onError: (e) => toast.err(ts("bk.toast.toggleFail", { e: String(e) })),
  });

  // ⟳ = 探活 + 重扫上游模型清单（diff 合并）。二次确认在 UI 把关，真操作走这里。
  const refreshBe = useMutation({
    mutationFn: (name: string) =>
      ipc<{
        healthy: boolean;
        /** 探活过了、但上游模型列表扫不出来（本地清单原样保留）。 */
        scan_failed: boolean;
        latency_ms: number;
        error: string | null;
        added: string[];
        removed: string[];
        kept: number;
      }>("refresh_backend", { name }),
    onSuccess: (r) => {
      refreshAll();
      if (!r.healthy) {
        toast.err(ts("bk.toast.probeFail", { e: r.error ?? t("bk.unknownReason") }));
      } else if (r.scan_failed) {
        // 探活过了、扫描没过：别再笼统报「刷新失败」，否则会被读成「后端连不上」。
        toast.err(
          ts("bk.toast.probeScanFail", {
            ms: r.latency_ms,
            e: r.error ?? t("bk.unknownReason"),
          }),
        );
      } else {
        toast.ok(
          ts("bk.toast.probeOk", {
            ms: r.latency_ms,
            added: r.added.length,
            removed: r.removed.length,
            kept: r.kept,
          }),
        );
      }
    },
    onError: (e) => toast.err(ts("bk.toast.refreshFail", { e: String(e) })),
  });

  // 删除后端：连带清掉它独占的参数覆盖与全局声明，服务端回报清了几条，这里如实转述。
  const deleteMut = useMutation({
    mutationFn: (name: string) => ipc<DeleteOutcome>("delete_backend", { name }),
    onSuccess: (r, name) => {
      // 展开态按 id 记的，后端没了那条记录也跟着作废；不清会留下幽灵展开键。
      const gone = backends.data?.find((b) => b.name === name)?.id;
      if (gone !== undefined) {
        setExpandedIds((m) => {
          const next = { ...m };
          delete next[gone];
          return next;
        });
      }
      refreshAll();
      toast.ok(
        r.dropped_params || r.dropped_meta
          ? ts("bk.toast.deletedWith", {
              base: ts("bk.toast.deleted", { name }),
              params: r.dropped_params,
              meta: r.dropped_meta,
            })
          : ts("bk.toast.deleted", { name }),
      );
    },
    onError: (e) => toast.err(ts("bk.toast.deleteFail", { e: String(e) })),
  });

  // url / key 原位编辑：保存即 upsert（key 的掩码占位符永不写回）。
  const inlineSave = useMutation({
    mutationFn: (v: { row: BackendRow; field: "url" | "key"; value: string }) => {
      const seed = seedFrom(v.row);
      const input: BackendInput =
        v.field === "url"
          ? { ...seed, base_url: v.value.replace(/\/+$/, "") }
          : { ...seed, api_key: v.value };
      // 原位编辑是「改这一行」：带上原名，后端才认作更新而不是撞名新增。
      return ipc("upsert_backend", { input: { ...input, original_name: v.row.name } });
    },
    onSuccess: (_r, v) => {
      toast.ok(v.field === "url" ? t("bk.toast.urlSaved") : t("bk.toast.keySaved"));
      refreshAll();
    },
    onError: (e) => toast.err(ts("bk.toast.saveFail", { e: String(e) })),
  });

  // 过滤（ui_01 回灌 U6）：维度与判据都在 useBackendFilter 里，页面只管摆控件。
  const bf = useBackendFilter(backends.data ?? []);
  const { rows } = bf;

  return (
    <div
      className={`w-full pt-[var(--content-pad-top)] ${
        tab === "console" ? "flex h-full flex-col" : "pb-10"
      }`}
    >
      {/* 顶层 Tab（纯文字+短横线下划线）。页头行已按用户 2026-11-19 截图拍板删除；
          「增加后端」主操作移到 Tab 行右侧，跟着「后端」页签走。 */}
      <div className="flex items-center border-b border-slate-800">
        <Tabs
          ariaLabel={t("bk.tabAria")}
          value={tab}
          onChange={(id) => setTab(id as "backends" | "params" | "json" | "console")}
          items={[
            {
              id: "backends",
              label: (
                <span className="inline-flex items-center gap-2">
                  <IconBackend width={16} height={16} />
                  {t("page.backends")}
                </span>
              ),
            },
            { id: "params", label: t("bk.tab.params") },
            { id: "json", label: "JSON" },
            { id: "console", label: t("bk.tab.console") },
          ]}
        />
        {tab === "backends" && (
          <>
            {/* 布局按用户 2026-11-21 截图拍板：「增加后端」是本页主操作，紧跟页签；
                搜索 + 状态分段是过滤工具，退到行尾右对齐。 */}
            <div className="pl-4 pb-1">
              <TextButton onClick={() => setAdding(true)}>{t("bk.add")}</TextButton>
            </div>
            <div className="ml-auto flex items-center pb-1">
              <BackendFilterBar bf={bf} />
            </div>
          </>
        )}
        {(tab === "console" || tab === "params") && (
          <div ref={setTabSlot} className="ml-auto flex min-w-0 items-center gap-2 pb-1" />
        )}
      </div>

      {tab === "backends" && (
        <div className="pt-3">
          <MigrationNote id="ia-2026-10">
            {tf("bk.ia", {
              params: <b>{t("bk.ia.params")}</b>,
              models: <b>{t("bk.ia.models")}</b>,
              settings: <b>{t("bk.ia.settings")}</b>,
            })}
          </MigrationNote>
        </div>
      )}

      {tab === "json" ? (
        <JsonTab />
      ) : tab === "params" ? (
        <div className="pt-4">
          <ParamsCenter initialBackend={paramsBackend} toolbarSlot={tabSlot} />
        </div>
      ) : tab === "console" ? (
        <div className="flex min-h-0 flex-1 flex-col pt-2">
          <ConsolePage pendingBackend={consoleBackend} toolbarSlot={tabSlot} />
        </div>
      ) : (
        <>
        {/* 摘要与过滤条都已上移到 Tab 行（用户 2026-11-16 / 11-20），不再各占一行。 */}

      {backends.isError && (
        <div className="pt-4">
          <QueryError inline title={t("bk.listError")} error={backends.error} onRetry={() => backends.refetch()} />
        </div>
      )}

      {/* 一整块浅底，行间弱分隔线（参考页的 slab，不是每张卡一块） */}
      <div className="mt-2 divide-y divide-slate-800 rounded-lg bg-slate-900/60">
        {backends.isLoading && <SkeletonRows rows={4} />}
        {/* 失败时不渲染「还没有后端」空态——否则红字错误和空态会同时出现、自相矛盾 */}
        {!backends.isLoading && !backends.isError && rows.length === 0 && (bf.filtered ? (
          <EmptyState
            icon={IconSearch}
            title={t("bk.emptyFiltered")}
            action={<ClearFiltersButton onClick={bf.clear} />}
          />
        ) : (
          <EmptyState icon={IconBackend} title={t("bk.emptyTitle")} desc={t("bk.emptyDesc")} />
        ))}
        {rows.map((b) => (
          <BackendRowItem
            key={b.id}
            b={b}
            busy={live.byBackend.has(b.name)}
            expanded={!!expandedIds[b.id]}
            onToggle={() => setExpandedIds((m) => ({ ...m, [b.id]: !m[b.id] }))}
            onRefresh={() => setRefreshTarget(b)}
            refreshing={refreshBe.isPending && refreshBe.variables === b.name}
            onEdit={() => setEditing(b)}
            onEnabled={(v) => toggleMut.mutate({ name: b.name, enabled: v })}
            managedCount={managedCount}
            vlmCount={vlmCountOf(b)}
            onSaveField={(field, value) => inlineSave.mutate({ row: b, field, value })}
            paramRows={paramRows}
            metrics={metricMap}
            cells={cellMap}
            clients={matrix.data?.clients ?? []}
            occ={occByApp}
            onOpenParams={(model) => setParamTarget({ backend: b.name, model })}
            onChanged={refreshAll}
            onDelete={() => setDeleteTarget(b)}
          />
        ))}
      </div>
        </>
      )}

      <BackendDialog open={adding} onClose={() => setAdding(false)} onDone={refreshAll} />
      <BackendDialog
        open={!!editing}
        initial={editing}
        onClose={() => setEditing(null)}
        onDone={refreshAll}
      />

      {refreshTarget && (
        <ConfirmDialog
          open
          title={t("bk.refresh.title")}
          onClose={() => setRefreshTarget(null)}
          onConfirm={() => refreshBe.mutate(refreshTarget.name)}
          confirmLabel={t("bk.refresh.confirm")}
        >
          {tf("bk.refresh.p1", {
            host: <b>{t("bk.refresh.upstreamTruth")}</b>,
            name: refreshTarget.name,
          })}
          <br />
          {tf("bk.refresh.p2", {
            drop: <Risk>{t("bk.refresh.detached")}</Risk>,
            keep: <b>{t("bk.refresh.kept")}</b>,
          })}
          <br />
          <span className="text-ink-4">{ts("bk.refresh.count", { n: refreshTarget.models.length })}</span>
        </ConfirmDialog>
      )}

      {deleteTarget && isProtectedHalogen(deleteTarget) && (
        /* 后端那道闸是硬拒（IPC/REST/JSON 写回三处），这里只负责把「为什么」说清楚：
           闸门在后端，UI 不能假装能删再让用户收一个红色报错。 */
        <ConfirmDialog
          open
          title={t("bk.protected.title")}
          confirmLabel={t("bk.protected.ok")}
          onClose={() => setDeleteTarget(null)}
          onConfirm={() => setDeleteTarget(null)}
        >
          {tf("bk.protected.p1", {
            rate: <b>{t("bk.protected.rate")}</b>,
            console: <b>{t("bk.protected.console")}</b>,
          })}
          <br />
          {tf("bk.protected.p2", {
            off: <Risk>{t("bk.protected.off")}</Risk>,
            gufo: <b>gufo</b>,
            yaml: <code>config.yaml</code>,
          })}
        </ConfirmDialog>
      )}

      {deleteTarget && !isProtectedHalogen(deleteTarget) && (
        <ConfirmDialog
          open
          danger
          title={ts("bk.delete.title", { name: deleteTarget.name })}
          irreversible
          onClose={() => setDeleteTarget(null)}
          onConfirm={() => deleteMut.mutate(deleteTarget.name)}
          confirmLabel={t("bk.delete.confirm")}
        >
          {t("bk.delete.p1")}
          <br />
          {tf("bk.delete.p2", { risk: <Risk>{t("bk.delete.noUndo")}</Risk> })}
          <br />
          <span className="text-ink-4">
            {ts("bk.refresh.count", { n: deleteTarget.models.length })}
            {deleteTarget.api_key ? t("bk.delete.keyCleared") : ""}
          </span>
        </ConfirmDialog>
      )}

      {paramTarget && (
        <ModelParamsDialog
          backend={paramTarget.backend}
          model={paramTarget.model}
          onClose={() => setParamTarget(null)}
        />
      )}

    </div>
  );
}

/** 行内编辑要把整行回填成 BackendInput（upsert 是全量语义）。 */
function seedFrom(row: BackendRow): BackendInput {
  return {
    ...row,
    api_key: row.api_key ?? "",
    webui_password: row.webui_password ?? "",
    models: [...row.models],
  };
}
