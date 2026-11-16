// 后端页（UI 重设计定稿，docs/ui_pages/01-后端页.md）。
// ①区行：闪烁点 + 名称 + url + 掩码 key ｜ 刷新(探活+重扫，二次确认) + 编辑 + 暴露开关
//   + webui + 删除；url / key 单击原位编辑（Enter 保存、Esc 或失焦放弃）。
//   删除有两条路：行尾这颗垃圾桶（显式退场），或摘光最后一个模型时连带删除。
// ②区：ModelRows 两行制（指标 + 代理分配）。顶层 Tab（后端/JSON）维持现状。
import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useToast } from "../components/Toast";
import { Tabs } from "../components/Tabs";
import { every, POLL } from "../lib/poll";
import { ParamsCenter } from "../components/ParamsCenter";
import { MigrationNote } from "../components/MigrationNote";
import { QueryError } from "../components/QueryError";
import { useLive } from "../lib/live";
import { BackendRowItem } from "./backends/BackendRowItem";
import { BackendDialog } from "./backends/BackendDialog";
import { JsonTab } from "../components/JsonTab";
import { ConfirmDialog, Risk } from "../components/ConfirmDialog";
import { isProtectedHalogen } from "../lib/backend";
import type { ParamRow } from "../lib/params";
import { ModelParamsDialog } from "../components/ModelParamsDialog";
import { TextButton, SkeletonRows } from "../components/ui";
import { IconBackend } from "../components/icons";
import { buildOccupancy } from "../components/model-rows/alloc";
import {
  ipc,
  type BackendInput,
  type BackendRow,
  type DeleteOutcome,
  type MatrixCell,
  type MatrixView,
  type ModelMetric,
} from "../lib/ipc";

export function BackendsPage({ paramsBackend }: { paramsBackend?: string }) {
  const qc = useQueryClient();
  const live = useLive();
  const [adding, setAdding] = useState(false);
  // 顶层 Tab：后端列表 / 参数中心 / JSON 直编。纯文字 + 激活项下短横线。
  // 参数中心原本是独立页「模型与参数」，而参数本来就是后端/模型的属性（决策 D3）。
  const [tab, setTab] = useState<"backends" | "params" | "json">("backends");
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
          `已启用 ${v.name}；halogen 与 gufo 同端口互斥，已自动停用 ${r.also_disabled.join("、")}`,
        );
      }
      refresh();
    },
    onError: (e) => toast.err(`切换失败：${e}`),
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
        toast.err(`探活失败：${r.error ?? "未知原因"}`);
      } else if (r.scan_failed) {
        // 探活过了、扫描没过：别再笼统报「刷新失败」，否则会被读成「后端连不上」。
        toast.err(
          `探活成功（${r.latency_ms}ms），但上游模型列表没扫出来：${r.error ?? "未知原因"}（本地清单原样保留）`,
        );
      } else {
        toast.ok(`探活成功 · ${r.latency_ms}ms · 新增 ${r.added.length} · 移除 ${r.removed.length} · 未变 ${r.kept}`);
      }
    },
    onError: (e) => toast.err(`刷新失败：${e}`),
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
        `已删除 ${name}` +
          (r.dropped_params || r.dropped_meta
            ? ` · 连带清理 参数覆盖 ${r.dropped_params} 条、模型声明 ${r.dropped_meta} 条`
            : ""),
      );
    },
    onError: (e) => toast.err(`删除失败：${e}`),
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
      toast.ok(v.field === "url" ? "URL 已保存" : "API Key 已覆盖");
      refreshAll();
    },
    onError: (e) => toast.err(`保存失败：${e}`),
  });

  const rows = backends.data ?? [];

  return (
    <div className="mx-auto w-full max-w-content pb-10 pt-0">
      {/* 分区首行：顶层 Tab（纯文字+短横线下划线）；右侧纯文字主操作只在后端 Tab 出现。
          不设 min-h-16：顶部紧贴应用标题栏，再撑高 64px 会和头部行叠出大片空白 */}
      <div className="flex items-end justify-between border-b border-slate-800">
        <div className="flex items-end gap-3">
          <Tabs
            ariaLabel="后端页分区"
            value={tab}
            onChange={(id) => setTab(id as "backends" | "params" | "json")}
            items={[
              {
                id: "backends",
                label: (
                  <span className="inline-flex items-center gap-2">
                    <IconBackend width={16} height={16} />
                    后端
                  </span>
                ),
              },
              { id: "params", label: "参数中心" },
              { id: "json", label: "JSON" },
            ]}
          />
          {/* 用户 2026-11-16：「N 个后端」摘要上移到 Tab 同一行，不再独占一行。 */}
          {tab === "backends" && (
            <span className="pb-2 text-xs text-ink-4">
              {rows.length} 个后端 · 流量按权重轮转分发
            </span>
          )}
        </div>
        {tab === "backends" && <TextButton onClick={() => setAdding(true)}>+ 增加后端</TextButton>}
      </div>

      {tab === "backends" && (
        <div className="pt-3">
          <MigrationNote id="ia-2026-10">
            信息架构调整：原「模型与参数」页已拆开 —— <b>参数中心</b>在本页上方 Tab，
            <b>模型清单</b>点行展开，各端槽位分配在「代理」页详情里；
            原「缓存」页并入本页下方的<b>设置</b>页。
          </MigrationNote>
        </div>
      )}

      {tab === "json" ? (
        <JsonTab />
      ) : tab === "params" ? (
        <div className="pt-4">
          <ParamsCenter initialBackend={paramsBackend} />
        </div>
      ) : (
        <>
        {/* 摘要已上移到 Tab 行（用户 2026-11-16），这里不再独占一行。 */}

      {backends.isError && (
        <div className="pt-4">
          <QueryError inline title="读取后端列表失败" error={backends.error} onRetry={() => backends.refetch()} />
        </div>
      )}

      {/* 一整块浅底，行间弱分隔线（参考页的 slab，不是每张卡一块） */}
      <div className="mt-2 divide-y divide-slate-800 rounded-lg bg-slate-900/60">
        {backends.isLoading && <SkeletonRows rows={4} />}
        {/* 失败时不渲染「还没有后端」空态——否则红字错误和空态会同时出现、自相矛盾 */}
        {!backends.isLoading && !backends.isError && rows.length === 0 && (
          <p className="px-4 py-6 text-xs text-ink-4">还没有后端 —— 点右上角「+ 增加后端」加一个。</p>
        )}
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
          title="刷新模型列表？"
          onClose={() => setRefreshTarget(null)}
          onConfirm={() => refreshBe.mutate(refreshTarget.name)}
          confirmLabel="确认刷新"
        >
          将向上游重新扫描并以<b>上游为准</b>覆盖「{refreshTarget.name}」的本地模型清单。
          <br />
          上游没有的模型会<Risk>暂时摘下</Risk>；其本机改名、参数覆盖、代理分配<b>保留</b>，
          上游恢复后自动挂回生效。
          <br />
          <span className="text-ink-4">当前 {refreshTarget.models.length} 个模型</span>
        </ConfirmDialog>
      )}

      {deleteTarget && isProtectedHalogen(deleteTarget) && (
        /* 后端那道闸是硬拒（IPC/REST/JSON 写回三处），这里只负责把「为什么」说清楚：
           闸门在后端，UI 不能假装能删再让用户收一个红色报错。 */
        <ConfirmDialog
          open
          title="halogen 后端不能删除"
          confirmLabel="知道了"
          onClose={() => setDeleteTarget(null)}
          onConfirm={() => setDeleteTarget(null)}
        >
          halogen 是<b>速率标定</b>（浮窗与各家速率扩展取原生速率的机位）和
          <b>halogen / gufo 控制台</b>的锚点，删掉这两块就空了。
          <br />
          要停它请用行上的<Risk>停用</Risk>开关，或在控制台把引擎切到 <b>gufo</b>；
          真要摘掉只能改 <code>config.yaml</code>（自己承担后果）。
        </ConfirmDialog>
      )}

      {deleteTarget && !isProtectedHalogen(deleteTarget) && (
        <ConfirmDialog
          open
          danger
          title={`删除后端「${deleteTarget.name}」？`}
          onClose={() => setDeleteTarget(null)}
          onConfirm={() => deleteMut.mutate(deleteTarget.name)}
          confirmLabel="确认删除"
        >
          该后端从网关下架，模型清单、别名、参数覆盖与缓存策略一并作废。
          <br />
          <Risk>应用内无法撤销</Risk>——真要再用得重新添加并重新配置一遍。
          <br />
          <span className="text-ink-4">
            当前 {deleteTarget.models.length} 个模型
            {deleteTarget.api_key ? " · 已存的 API Key 一并清除" : ""}
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
