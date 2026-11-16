/**
 * 参数中心（计划 4.1 / 决策 D3）：请求 > 模型 > 全局 > 后端 的参数覆盖编辑。
 *
 * 原先它和 claude 角色表合在一张独立页「模型与参数」上，而边条里根本没有这一格 ——
 * 页面进不去、内容又分属两个域。现在参数中心并入后端页顶部 Tab（参数本来就是后端/模型的属性），
 * 角色表并入代理页（那是「哪个客户端用哪个模型」的分配问题，归代理）。
 */
import { useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useToast } from "../components/Toast";
import { QueryError } from "../components/QueryError";
import { ConfirmDialog } from "../components/ConfirmDialog";
import { Badge, Card, barSelectCls } from "../components/ui";
import { tableCls, theadCls } from "../components/Table";
import { ModelParamsDialog } from "../components/ModelParamsDialog";
import { Tabs } from "../components/Tabs";
import { TEMPLATES, applyPushReport, type PushReport } from "../lib/params";
import { AddRow } from "./params-center/AddRow";
import type { CatalogEntry, ParamRow, Scope } from "./params-center/types";
import { ipc, type BackendRow } from "../lib/ipc";


export function ParamsCenter({ initialBackend }: { initialBackend?: string }) {
  const qc = useQueryClient();
  const [scope, setScope] = useState<Scope>("model");
  const [backend, setBackend] = useState(initialBackend ?? "");
  const [model, setModel] = useState("");
  const toast = useToast();
  // 删覆盖会让参数回落到继承链，属于「改了配置」，闸门矩阵里给轻量确认。
  const [delRow, setDelRow] = useState<{ id: number; key: string } | null>(null);
  const [dialog, setDialog] = useState<{ backend: string; model: string } | null>(null);

  const backends = useQuery({
    queryKey: ["backends"],
    queryFn: () => ipc<BackendRow[]>("list_backends"),
  });
  const params = useQuery({
    queryKey: ["params"],
    // IPC 返回裸数组（REST 才包 {overrides}）。
    queryFn: () => ipc<ParamRow[]>("list_param_overrides"),
  });
  const catalog = useQuery({
    queryKey: ["param-catalog"],
    queryFn: () => ipc<CatalogEntry[]>("get_param_catalog"),
  });

  const rows = params.data ?? [];
  const bs = backends.data ?? [];
  const activeBackend = backend || bs[0]?.name || "";
  const models = bs.find((b) => b.name === activeBackend)?.models ?? [];
  const activeModel = model || models[0] || "";
  const caps = useQuery({
    queryKey: ["backend-caps", activeBackend],
    queryFn: () =>
      ipc<{ engine: string | null; keys: string[] }>("get_backend_capabilities", {
        backend: activeBackend,
      }),
    enabled: scope !== "global" && !!activeBackend,
  });

  const refId = useMemo(() => {
    if (scope === "global") return "";
    if (scope === "backend") return activeBackend;
    return `${activeBackend}:${activeModel}`;
  }, [scope, activeBackend, activeModel]);

  const visible = rows.filter((r) => r.scope === scope && r.ref_id === refId);

  // 新版（U-B）后模型清单已收进后端页展开区，这里只剩参数中心的作用域选择。

  const refresh = () => qc.invalidateQueries({ queryKey: ["params"] });

  const save = useMutation({
    mutationFn: (v: { key: string; value: string; policy: string }) => {
      let parsed: unknown;
      try {
        parsed = JSON.parse(v.value);
      } catch {
        parsed = v.value; // 裸字符串当纯文本
      }
      return ipc<PushReport | null>("upsert_param_override", {
        input: { scope, ref_id: refId, key: v.key, value: parsed, policy: v.policy },
      });
    },
    onSuccess: (report, v) => {
      // 模型层会顺手下发到后端本体，战报说真话；其它层 report 为 null 走原话术。
      applyPushReport(toast, report, "save", v.key, () => toast.ok("已保存，即刻生效"));
      refresh();
    },
    onError: (e) => toast.err(`保存失败：${e}`),
  });

  const remove = useMutation({
    mutationFn: (row: { id: number; key: string }) =>
      ipc<PushReport | null>("delete_param_override", { id: row.id }),
    onSuccess: (report, row) => {
      applyPushReport(toast, report, "delete", row.key, () => {});
      refresh();
    },
    onError: (e) => toast.err(`删除失败：${e}`),
  });

  return (
    <div className="space-y-4">
      {dialog && (
        <ModelParamsDialog
          backend={dialog.backend}
          model={dialog.model}
          onClose={() => setDialog(null)}
        />
      )}
      <div className="flex items-center gap-2 text-sm">
        <Tabs
          variant="segmented"
          ariaLabel="参数作用域"
          value={scope}
          onChange={(id) => setScope(id as Scope)}
          items={[
            { id: "global", label: "全局" },
            { id: "backend", label: "后端" },
            { id: "model", label: "模型" },
          ]}
        />
        {scope !== "global" && (
          <select
            value={activeBackend}
            onChange={(e) => {
              setBackend(e.target.value);
              setModel("");
            }}
            className={barSelectCls + " text-xs"}
          >
            {bs.map((b) => (
              <option key={b.name}>{b.name}</option>
            ))}
          </select>
        )}
        {scope === "model" && (
          <select
            value={activeModel}
            onChange={(e) => setModel(e.target.value)}
            className={barSelectCls + " text-xs"}
          >
            {models.map((m) => (
              <option key={m}>{m}</option>
            ))}
          </select>
        )}
        {backends.isError && (
          <span className="ml-auto text-xs text-ink-4">
            {/* 后端清单读失败时作用域下拉是空的：不给提示用户会以为「没有后端」 */}
            <QueryError inline title="后端清单读取失败" error={backends.error} onRetry={() => backends.refetch()} />
          </span>
        )}
      </div>

      <Card>
        <table className={tableCls}>
          <thead className={theadCls}>
            <tr>
              <th className="px-3 py-2">参数键</th>
              <th className="px-3 py-2">值（JSON）</th>
              <th className="px-3 py-2">策略</th>
              <th className="px-3 py-2" />
            </tr>
          </thead>
          <tbody>
            {/* 失败 ≠ 此作用域没覆盖：红字 + 重试，别让用户以为配置是空的 */}
            {params.isError && (
              <tr>
                <td colSpan={4} className="px-3 py-6 text-center">
                  <QueryError
                    title="读取参数覆盖失败"
                    error={params.error}
                    onRetry={() => params.refetch()}
                  />
                </td>
              </tr>
            )}
            {!params.isError && visible.length === 0 && (
              <tr>
                <td colSpan={4} className="px-3 py-6 text-center text-ink-4">
                  此作用域暂无参数覆盖
                </td>
              </tr>
            )}
            {visible.map((r) => (
              <tr key={r.id} className="border-t border-slate-800">
                <td className="px-3 py-2 font-mono">{r.key}</td>
                <td className="px-3 py-2 font-mono text-ink-2">
                  {JSON.stringify(r.value)}
                </td>
                <td className="px-3 py-2">
                  <Badge>{r.policy}</Badge>
                </td>
                <td className="px-3 py-2 text-right">
                  <button
                    onClick={() => setDelRow({ id: r.id, key: r.key })}
                    className="text-ink-4 hover:text-[var(--sem-danger-text)]"
                  >
                    删除
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </Card>

      {delRow && (
        <ConfirmDialog
          open
          title={`删除参数覆盖 ${delRow.key}？`}
          onClose={() => setDelRow(null)}
          onConfirm={() => remove.mutate({ id: delRow.id, key: delRow.key })}
          confirmLabel="确认删除"
        >
          删除后 <b>{delRow.key}</b> 回落到上层继承值（后端默认 → 引擎自决），
          已在途的请求不受影响。
        </ConfirmDialog>
      )}

      {scope !== "global" && (
        <Card>
          <div className="flex flex-wrap items-center gap-2 p-3 text-xs">
            <span className="text-ink-4">
              模板套用{caps.data?.engine ? `（引擎 ${caps.data.engine}）` : ""}
            </span>
            {TEMPLATES.map((t) => (
              <button
                key={t.name}
                onClick={() =>
                  t.params.forEach(([k, v, pol]) =>
                    save.mutate({ key: k, value: v, policy: pol }),
                  )
                }
                className="rounded-md border border-slate-700 px-2 py-1 text-ink-2 hover:border-cyan-600"
              >
                {t.name}
              </button>
            ))}
            {caps.data && (
              <span className="ml-auto text-ink-4">
                支持 {caps.data.keys.length} 个目录键
              </span>
            )}
          </div>
        </Card>
      )}

      <Card>
        <AddRow
          onSave={(v) => save.mutate(v)}
          saving={save.isPending}
          catalog={catalog.data ?? []}
          supported={caps.data?.keys ?? null}
          gate={scope !== "global"}
          backendName={activeBackend}
        />
      </Card>

    </div>
  );
}
