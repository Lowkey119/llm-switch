// 日志页：运行日志（logs 表）过滤查看 + 实时 tail + 导出 + 级别热改。
import { Fragment, useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useToast } from "../components/Toast";
import { ConfirmDialog, Risk } from "../components/ConfirmDialog";
import { Badge, Card, barSelectCls } from "../components/ui";
import { theadCls } from "../components/Table";
import { TableRow } from "../components/Row";
import { QueryError } from "../components/QueryError";
import { SearchInput } from "../components/SearchInput";
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
    onError: (e) => toast.err(`导出失败：${String(e)}`),
  });

  const changeLevel = useMutation({
    mutationFn: (lv: string) => ipc("set_log_level", { level: lv }),
    onSuccess: () => {
      toast.ok("日志级别已更新");
      qc.invalidateQueries({ queryKey: ["log-level"] });
    },
    onError: (e) => toast.err(`改日志级别失败：${String(e)}`),
  });

  const purge = useMutation({
    mutationFn: () =>
      ipc("logs_purge", { beforeTs: Date.now() - 24 * 3600 * 1000 }),
    onSuccess: () => {
      toast.ok("已清理 24 小时前的日志");
      qc.invalidateQueries({ queryKey: ["logs"] });
      // 清理会删掉老后端 / 老模型的行，过滤下拉的选项要跟着缩
      qc.invalidateQueries({ queryKey: ["log-facets"] });
    },
    onError: (e) => toast.err(`清理失败：${String(e)}`),
  });

  const rows = logs.data ?? [];
  return (
    <div className="flex h-full flex-col gap-3">
      {/* UI-16（用户 2026-11-16，附截图）：过滤条、导出/清理、当前级别合并为一行，
          各自收窄宽度适配；窄窗下 flex-wrap 兜底换行。 */}
      <div className="flex flex-wrap items-center gap-2 text-xs">
        <select
          value={level}
          onChange={(e) => setLevel(e.target.value)}
          className={barSelectCls}
        >
          <option value="">全部级别</option>
          <option value="info">info</option>
          <option value="warn">warn</option>
          <option value="error">error</option>
        </select>
        <select
          value={backend}
          onChange={(e) => setBackend(e.target.value)}
          className={barSelectCls}
        >
          <option value="">全部后端</option>
          {(facets.data?.backends ?? []).map((b) => (
            <option key={b} value={b}>
              {b}
            </option>
          ))}
        </select>
        <select
          value={model}
          onChange={(e) => setModel(e.target.value)}
          className={barSelectCls + " max-w-40"}
        >
          <option value="">全部模型</option>
          {(facets.data?.models ?? []).map((m) => (
            <option key={m} value={m}>
              {m}
            </option>
          ))}
        </select>
        <SearchInput
          value={text}
          onChange={setText}
          placeholder="搜索消息 / 短码 / request_id"
          inputClassName="w-44"
          debounce={250}
        />
        <label className="flex items-center gap-1 text-ink-3">
          <input
            type="checkbox"
            checked={onlyErrors}
            onChange={(e) => setOnlyErrors(e.target.checked)}
          />
          仅错误
        </label>
        <label className="flex items-center gap-1 text-ink-3">
          <input type="checkbox" checked={auto} onChange={(e) => setAuto(e.target.checked)} />
          实时 tail
        </label>
        <span className="text-ink-4">{rows.length} 条</span>
        <label className="flex items-center gap-1 text-ink-4">
          当前级别
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
          className="ml-auto text-ink-4 hover:text-cyan-300"
          title="按当前过滤条件导出 NDJSON"
        >
          导出
        </button>
        <button
          onClick={() => setConfirmPurge(true)}
          className="text-ink-4 hover:text-[var(--sem-danger-text)]"
          title="清理 24 小时前的日志"
        >
          清理旧日志
        </button>
      </div>

      {confirmPurge && (
        <ConfirmDialog
          open
          title="清理 24 小时前的日志？"
          onClose={() => setConfirmPurge(false)}
          onConfirm={() => purge.mutate()}
          confirmLabel="确认清理"
          danger
        >
          <Risk>24 小时前的日志行会被永久删除，不可恢复</Risk>，
          之后按当前过滤条件也导出不到它们。当前列表 {rows.length} 条。
        </ConfirmDialog>
      )}

      {/* mutation 失败提示：导出/清理/改级别原先失败时界面毫无反应 */}

      <Card className="min-h-0 flex-1 overflow-auto">
        <table className="w-full text-left font-mono text-xs">
          <thead className={theadCls}>
            <tr>
              <th className="px-2 py-1.5">时间</th>
              <th className="px-2 py-1.5">级别</th>
              <th className="px-2 py-1.5">短码</th>
              <th className="px-2 py-1.5">后端/模型</th>
              <th className="px-2 py-1.5">消息</th>
              <th className="px-2 py-1.5">附加</th>
            </tr>
          </thead>
          <tbody>
            {/* 读取失败要显示失败（可重试），不能渲染成「暂无日志」——两者必须能区分 */}
            {logs.isError && (
              <tr>
                <td colSpan={6} className="px-3 py-6">
                  <QueryError title="读取日志失败" error={logs.error} onRetry={() => logs.refetch()} />
                </td>
              </tr>
            )}
            {rows.map((r) => (
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
            {!logs.isError && rows.length === 0 && (
              <tr>
                <td colSpan={6} className="px-3 py-8 text-center text-ink-4">
                  暂无日志
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </Card>
    </div>
  );
}
