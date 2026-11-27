// 日志页：运行日志（logs 表）过滤查看 + 实时 tail + 导出 + 级别热改。
import { Fragment, useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useToast } from "../components/Toast";
import { ConfirmDialog, Risk } from "../components/ConfirmDialog";
import { Badge, Card, barSelectCls } from "../components/ui";

import { t, tf, ts } from "../lib/i18n";
import { useSort, sortRows, SortTh } from "../components/Table";
import type { SortState } from "../components/Table";
import { TableRow } from "../components/Row";
import { QueryError } from "../components/QueryError";
import { SearchInput } from "../components/SearchInput";
import { EmptyCell } from "../components/States";
import { ClearFiltersButton, FilterBar } from "../components/FilterBar";
import type { FilterTag } from "../components/FilterBar";
import { Tabs } from "../components/Tabs";
import { ipc } from "../lib/ipc";
import { every, POLL } from "../lib/poll";

interface LogRow {
  id: number;
  ts: number;
  level: string;
  code: string | null;
  message: string;
  request_id: string | null;
  backend: string | null;
  model: string | null;
  extra: unknown;
}

/** 日志表的可排序列（ui_01 回灌 U4）：短码 / 消息 / 附加排序没读法，不开。 */
type LogKey = "ts" | "level" | "backend";

const LEVEL_COLOR: Record<string, string> = {
  info: "text-ink-2",
  warn: "text-[var(--sem-busy-text)]",
  error: "text-[var(--sem-danger-text)]",
};

export function LogsPage() {
  const qc = useQueryClient();
  const [level, setLevel] = useState("");
  const [text, setText] = useState("");
  const [onlyErrors, setOnlyErrors] = useState(false);
  const [backend, setBackend] = useState("");
  const [model, setModel] = useState("");
  const [auto, setAuto] = useState(true);
  const [openId, setOpenId] = useState<number | null>(null);
  const [logLevel, setLogLevel] = useState<string | null>(null);
  // 这几个按钮型 mutation 原先全没有 onError：失败时界面毫无反应，像按钮没点上。
  const toast = useToast();
  // 清理是不可逆动作，闸门矩阵要求先说清后果再动手。
  const [confirmPurge, setConfirmPurge] = useState(false);

  const levelQ = useQuery({
    queryKey: ["log-level"],
    queryFn: () => ipc<string>("get_log_level"),
  });
  useEffect(() => {
    if (levelQ.data && logLevel === null) setLogLevel(levelQ.data);
  }, [levelQ.data, logLevel]);

  // 实时 tail：GUI 内 1s 合并推流，收到即刷新查询（单一数据源，不双轨维护）。
  useEffect(() => {
    if (!auto) return;
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    import("@tauri-apps/api/event")
      .then(({ listen }) =>
        listen("logs://tail", () => {
          if (!cancelled) qc.invalidateQueries({ queryKey: ["logs"] });
        }),
      )
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      })
      .catch(() => {});
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [auto, qc]);

  // 两个过滤下拉的选项：日志真值里出现过的后端 / 模型（DISTINCT）——历史行的
  // 后端可能早已删除，选项不能从配置表推。30s 刷新够用（新后端不会每秒出现）。
  const facets = useQuery({
    queryKey: ["log-facets"],
    queryFn: () => ipc<{ backends: string[]; models: string[] }>("logs_facets"),
    refetchInterval: () => every(POLL.slow),
  });

  const logs = useQuery({
    queryKey: ["logs", level, text, onlyErrors, backend, model],
    queryFn: () =>
      ipc<LogRow[]>("logs_query", {
        input: {
          level: level || null,
          backend: backend || null,
          model: model || null,
          text: text || null,
          only_errors: onlyErrors,
          limit: 300,
        },
      }),
    refetchInterval: () => (auto ? every(POLL.tail) : false),
  });

  const exportNdjson = useMutation({
    mutationFn: () =>
      ipc<string>("logs_export", {
        input: {
          level: level || null,
          backend: backend || null,
          model: model || null,
          text: text || null,
          only_errors: onlyErrors,
          limit: 100000,
        },
      }),
    onSuccess: (nd) => {
      const blob = new Blob([nd], { type: "application/x-ndjson" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = `llm-switch-logs-${new Date().toISOString().slice(0, 10)}.ndjson`;
      a.click();
      URL.revokeObjectURL(url);
    },
    onError: (e) => toast.err(ts("logs.exportFail", { e: String(e) })),
  });

  const changeLevel = useMutation({
    mutationFn: (lv: string) => ipc("set_log_level", { level: lv }),
    onSuccess: () => {
      toast.ok(t("logs.levelUpdated"));
      qc.invalidateQueries({ queryKey: ["log-level"] });
    },
    onError: (e) => toast.err(ts("logs.levelFail", { e: String(e) })),
  });

  const purge = useMutation({
    mutationFn: () =>
      ipc("logs_purge", { beforeTs: Date.now() - 24 * 3600 * 1000 }),
    onSuccess: () => {
      toast.ok(t("logs.purgedOk"));
      qc.invalidateQueries({ queryKey: ["logs"] });
      // 清理会删掉老后端 / 老模型的行，过滤下拉的选项要跟着缩
      qc.invalidateQueries({ queryKey: ["log-facets"] });
    },
    onError: (e) => toast.err(ts("logs.purgeFail", { e: String(e) })),
  });

  const rows = logs.data ?? [];
  // 空态分两种（U5）：筛没了 vs 真没日志。两者共用一句「暂无日志」就是骗人——
  // 用户会以为系统没在记，而其实是他自己的筛选把行滤干了。
  const filtered = !!(level || text.trim() || onlyErrors || backend || model);
  // 已选条件 chip（U6）：每个生效条件一颗，各自能单清；一个都没就不占行。
  const activeTags = [
    text.trim() ? { key: "q", label: t("filter.tagSearch"), value: text.trim(), clear: () => setText("") } : null,
    level ? { key: "level", label: t("filter.tagLevel"), value: level, clear: () => setLevel("") } : null,
    backend ? { key: "backend", label: t("filter.tagBackend"), value: backend, clear: () => setBackend("") } : null,
    model ? { key: "model", label: t("filter.tagModel"), value: model, clear: () => setModel("") } : null,
    onlyErrors ? { key: "err", label: t("filter.tagOnlyErr"), clear: () => setOnlyErrors(false) } : null,
  ].filter(Boolean) as FilterTag[];
  const clearFilters = () => {
    setLevel("");
    setText("");
    setOnlyErrors(false);
    setBackend("");
    setModel("");
  };
  // 排序（ui_01 回灌 U4）：初始不设排序 = 保持后端给的序（新的在前），
  // 这里一上来就写个 desc 会把后端已经排好的序伪装成是本页排的。
  const logSort = useSort<LogKey>();
  const view = sortRows(rows, logSort.sort as SortState<string>, (r, k) => {
    if (k === "ts") return r.ts;
    if (k === "level") return r.level;
    return r.backend ?? r.model;
  });
  return (
    <div className="flex h-full flex-col gap-3 pt-[var(--content-pad-top)]">
      {/* UI-16（用户 2026-11-16，附截图）：过滤条各自收窄宽度适配；窄窗下 flex-wrap 兜底换行。
          实时 tail / 当前级别 / 导出 / 清理并入过滤条 trailing 槽（用户 2026-11-20 拍板：
          全局控制独占一行右对齐悬空，与过滤条、表头堆成三层）——trailing 本就是
          「不是过滤的页面动作」的槽位（见 FilterBar 注释），ml-auto 顶到行尾。 */}
      <FilterBar
        count={rows.length}
        active={activeTags}
        onClearAll={filtered ? clearFilters : undefined}
        trailing={
          <>
            <label className="flex items-center gap-1 text-ink-3">
              <input type="checkbox" checked={auto} onChange={(e) => setAuto(e.target.checked)} />
              {t("logs.auto")}
            </label>
            <label className="flex items-center gap-1 text-ink-4">
              {t("logs.curLevel")}
              <select
                value={logLevel ?? ""}
                onChange={(e) => {
                  setLogLevel(e.target.value);
                  changeLevel.mutate(e.target.value);
                }}
                className="rounded border border-slate-700 bg-slate-900 px-1 py-0.5 text-ink-2"
              >
                {["error", "warn", "info", "debug", "trace"].map((lv) => (
                  <option key={lv} value={lv}>
                    {lv}
                  </option>
                ))}
              </select>
            </label>
            <button
              onClick={() => exportNdjson.mutate()}
              className="text-ink-4 hover:text-cyan-300"
              title={t("logs.exportTip")}
            >
              {t("logs.export")}
            </button>
            <button
              onClick={() => setConfirmPurge(true)}
              className="text-ink-4 hover:text-[var(--sem-danger-text)]"
              title={t("logs.purgeTip")}
            >
              {t("logs.purge")}
            </button>
          </>
        }
      >
        <SearchInput
          value={text}
          onChange={setText}
          placeholder={t("logs.searchPh")}
          ariaLabel={ts("logs.searchAria")}
          inputClassName="w-44"
          debounce={250}
        />
        <Tabs
          variant="segmented"
          size="sm"
          ariaLabel={t("filter.byLevel")}
          value={level}
          onChange={setLevel}
          items={[
            { id: "", label: t("filter.allLevels") },
            { id: "info", label: "info" },
            { id: "warn", label: "warn" },
            { id: "error", label: "error" },
          ]}
        />
        <select value={backend} onChange={(e) => setBackend(e.target.value)} className={barSelectCls}>
          <option value="">{t("filter.allBackends")}</option>
          {(facets.data?.backends ?? []).map((b) => (
            <option key={b} value={b}>
              {b}
            </option>
          ))}
        </select>
        <select
          value={model}
          onChange={(e) => setModel(e.target.value)}
          className={barSelectCls + " max-w-32"}
        >
          <option value="">{t("filter.allModels")}</option>
          {(facets.data?.models ?? []).map((m) => (
            <option key={m} value={m}>
              {m}
            </option>
          ))}
        </select>
        <label className="flex items-center gap-1 text-ink-3">
          <input type="checkbox" checked={onlyErrors} onChange={(e) => setOnlyErrors(e.target.checked)} />
          {t("logs.onlyErrors")}
        </label>
      </FilterBar>

      {confirmPurge && (
        <ConfirmDialog
          open
          title={t("logs.purgeTitle")}
          irreversible
          onClose={() => setConfirmPurge(false)}
          onConfirm={() => purge.mutate()}
          confirmLabel={t("logs.purgeConfirm")}
          danger
        >
          {tf("logs.purgeBody", {
            risk: <Risk>{t("logs.purgeRisk")}</Risk>,
            n: rows.length,
          })}
        </ConfirmDialog>
      )}

      {/* mutation 失败提示：导出/清理/改级别原先失败时界面毫无反应 */}

      {/* 表头物理固定（用户 2026-11-20 两轮复核截图：th 上的 sticky 在 WKWebView 始终不生效，
          border-collapse / border-separate 都压不住——WebKit 表格单元 sticky 是历史缺陷）。
          不再依赖 sticky：表头 = 滚动区外的独立 table，表体在下方独立滚动；
          两表 table-fixed + 同一 colgroup 保证列对齐（macOS 悬浮滚动条不占宽，无错位）。 */}
      <Card className="flex min-h-0 flex-1 flex-col overflow-hidden p-0">
        <div className="shrink-0 border-b border-slate-800 bg-slate-900 text-ink-4">
          <table className="w-full table-fixed border-separate border-spacing-0 text-left font-mono text-xs">
            <colgroup>
              <col className="w-[84px]" />
              <col className="w-[48px]" />
              <col className="w-[150px]" />
              <col className="w-[130px]" />
              <col />
              <col />
            </colgroup>
            <thead>
              <tr>
                <SortTh label={t("logs.th.ts")} k="ts" sort={logSort.sort} onSort={logSort.toggle} className="px-2 py-1.5" />
                <SortTh label={t("logs.th.level")} k="level" sort={logSort.sort} onSort={logSort.toggle} className="px-2 py-1.5" />
                <th className="px-2 py-1.5">{t("logs.th.code")}</th>
                <SortTh label={t("logs.th.backend")} k="backend" sort={logSort.sort} onSort={logSort.toggle} className="px-2 py-1.5" />
                <th className="px-2 py-1.5">{t("logs.th.msg")}</th>
                <th className="px-2 py-1.5">{t("logs.th.extra")}</th>
              </tr>
            </thead>
          </table>
        </div>
        <div className="min-h-0 flex-1 overflow-auto">
        <table className="w-full table-fixed text-left font-mono text-xs">
          <colgroup>
            <col className="w-[84px]" />
            <col className="w-[48px]" />
            <col className="w-[150px]" />
            <col className="w-[130px]" />
            <col />
            <col />
          </colgroup>
          <tbody>
            {/* 读取失败要显示失败（可重试），不能渲染成「暂无日志」——两者必须能区分 */}
            {logs.isError && (
              <tr>
                <td colSpan={6} className="px-3 py-4">
                  {/* 表格单元里用 inline：块状红框套在卡片里就是「卡中卡」，
                      impeccable 报的 nested-cards 就是它；inline 一行带重试，同样说清失败了。 */}
                  <QueryError inline title={t("logs.loadFail")} error={logs.error} onRetry={() => logs.refetch()} />
                </td>
              </tr>
            )}
            {view.map((r) => (
              <Fragment key={r.id}>
              <TableRow
                expanded={openId === r.id}
                onClick={() => setOpenId(openId === r.id ? null : r.id)}
                className="border-t border-slate-800/60 align-top hover:bg-slate-800/30"
              >
                <td className="whitespace-nowrap px-2 py-1 text-ink-4">
                  {new Date(r.ts).toLocaleTimeString()}
                </td>
                <td className={`px-2 py-1 ${LEVEL_COLOR[r.level] ?? ""}`}>{r.level}</td>
                <td className="px-2 py-1">
                  <Badge>{r.code ?? "—"}</Badge>
                </td>
                <td className="px-2 py-1 text-ink-3">
                  {r.backend ?? "—"}
                  {r.model ? ` · ${r.model}` : ""}
                </td>
                <td className="px-2 py-1 text-ink-2">
                  {r.message}
                  {r.request_id && (
                    <span className="ml-2 text-ink-4">{r.request_id.slice(0, 8)}</span>
                  )}
                </td>
                <td className="px-2 py-1 text-ink-4">
                  {r.extra ? JSON.stringify(r.extra) : ""}
                </td>
              </TableRow>
              {openId === r.id && (
                <tr className="bg-slate-900/40">
                  <td colSpan={6} className="px-3 py-2 font-mono text-xs text-ink-3">
                    <div className="flex flex-wrap gap-3">
                      {r.request_id && (
                        <button
                          className="text-cyan-400 hover:underline"
                          onClick={(e) => {
                            e.stopPropagation();
                            navigator.clipboard.writeText(r.request_id!);
                          }}
                        >
                          复制 request_id
                        </button>
                      )}
                      <span className="text-ink-4">ts={new Date(r.ts).toLocaleString()}</span>
                    </div>
                    {r.extra ? (
                      <pre className="mt-1 whitespace-pre-wrap text-ink-4">
                        {JSON.stringify(r.extra, null, 1)}
                      </pre>
                    ) : null}
                  </td>
                </tr>
              )}
              </Fragment>
            ))}
            {/* 空态只在「请求成功且没数据」时出现；失败走上方的错误行 */}
            {!logs.isError && rows.length === 0 && (filtered ? (
              <EmptyCell cols={6}>
                {t("logs.emptyFiltered")}
                <ClearFiltersButton onClick={clearFilters} className="ml-2" />
              </EmptyCell>
            ) : (
              <EmptyCell cols={6}>{t("logs.empty")}</EmptyCell>
            ))}
          </tbody>
        </table>
        </div>
      </Card>
    </div>
  );
}
