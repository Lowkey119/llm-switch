/**
 * 响应缓存卡（计划 4.1 / 决策 D3）：策略矩阵、命中效果与容量水位。
 *
 * 原先它是一整页，但边条里没有入口（P1-7 的孤儿页之一），而且内容本质是
 * 「网关的一个开关 + 它的效果读数」，归设置。现在作为设置页里的一张卡存在。
 */
import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { QueryError } from "../components/QueryError";
import { useToast } from "../components/Toast";
import { ConfirmDialog, Risk } from "../components/ConfirmDialog";
import { Button, Card, Toggle, fieldCls } from "../components/ui";
import { ipc, type ModelMeta } from "../lib/ipc";
import { every, POLL } from "../lib/poll";
import { TierCard } from "./cache/TierCard";
import { theadCls } from "./Table";

interface CacheStats {
  hits: number;
  misses: number;
  hit_rate: number;
  bypass: number;
  saved_input_tokens: number;
  saved_output_tokens: number;
  entries: number;
  memory_bytes: number;
  memory_budget_bytes: number;
  disk_bytes: number;
  disk_budget_bytes: number;
  enabled_globally: boolean;
}

interface PolicyRow {
  scope: string; // "global" | "model:<名>"
  policy: {
    enabled?: boolean;
    ttl?: number;
    key_granularity?: string;
    replay?: string;
  };
}

const fmtBytes = (n: number) => {
  if (n <= 0) return "0 B";
  const u = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.min(Math.floor(Math.log(n) / Math.log(1024)), u.length - 1);
  return `${(n / 1024 ** i).toFixed(i ? 1 : 0)} ${u[i]}`;
};

export function CacheCard() {
  const qc = useQueryClient();
  const stats = useQuery({
    queryKey: ["cache-stats"],
    queryFn: () => ipc<CacheStats>("get_cache_stats"),
    refetchInterval: () => every(POLL.fast),
  });
  const policies = useQuery({
    queryKey: ["cache-policies"],
    queryFn: () => ipc<PolicyRow[]>("list_cache_policies"),
  });
  // 已知模型清单：给「按模型策略」的下拉用，避免手敲模型名敲错。
  const modelMeta = useQuery({
    queryKey: ["model-meta"],
    queryFn: () => ipc<ModelMeta[]>("model_meta_get"),
  });

  const invalidate = () => {
    qc.invalidateQueries({ queryKey: ["cache-stats"] });
    qc.invalidateQueries({ queryKey: ["cache-policies"] });
  };

  // mutation 失败必须有反馈：不显示的话，删除/清空点了没反应，与成功无法区分。
  const toast = useToast();
  // 闸门矩阵：清空全部缓存不可逆（丢掉全部命中收益），删策略回到继承，两者都要先确认。
  const [confirmPurgeAll, setConfirmPurgeAll] = useState(false);
  const [confirmDelPolicy, setConfirmDelPolicy] = useState<string | null>(null);

  const setPolicy = useMutation({
    mutationFn: (vars: { scope: string; policy: unknown }) =>
      ipc("set_cache_policy", { scope: vars.scope, policy: vars.policy }),
    onSuccess: invalidate,
    onError: (e) => toast.err(`保存策略失败：${e}`),
  });
  const delPolicy = useMutation({
    mutationFn: (scope: string) => ipc("delete_cache_policy", { scope }),
    onSuccess: invalidate,
    onError: (e) => toast.err(`删除策略失败：${e}`),
  });
  const purge = useMutation({
    mutationFn: (model?: string) => ipc<number>("purge_cache", { model, all: !model }),
    onSuccess: invalidate,
    onError: (e) => toast.err(`清空缓存失败：${e}`),
  });

  const s = stats.data;
  const globalRow = policies.data?.find((p) => p.scope === "global");
  const modelRows = (policies.data ?? []).filter((p) => p.scope.startsWith("model:"));
  const [newModel, setNewModel] = useState("");
  // 下拉候选：已知模型去掉已添加的（已添加的在表里就地改，不重复加）。
  const addedModels = new Set(modelRows.map((r) => r.scope.slice("model:".length)));
  const candidateModels = (modelMeta.data ?? [])
    .map((m) => m.model)
    .filter((m) => !addedModels.has(m))
    .sort();

  return (
    <div className="space-y-4">
      {/* stats 读取失败要露出来：否则四张卡全显示「—」，与加载中无从区分 */}
      {stats.isError && (
        <QueryError title="读取缓存统计失败" error={stats.error} onRetry={() => stats.refetch()} />
      )}
      {/* 效果卡：列数随窗口自适应（同 Usage 统计卡口径），定死 4 列在窄窗下
          sub 长文案（「输入 … · 输出 …」）会折行挤压。 */}
      <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
        <Mini
          label="命中率"
          value={s ? `${(s.hit_rate * 100).toFixed(1)}%` : "—"}
          sub={s ? `${s.hits} 命中 / ${s.misses} 未命中 / ${s.bypass} 绕行` : undefined}
        />
        <Mini
          label="节省 tokens"
          value={s ? (s.saved_input_tokens + s.saved_output_tokens).toLocaleString() : "—"}
          sub={s ? `输入 ${s.saved_input_tokens.toLocaleString()} · 输出 ${s.saved_output_tokens.toLocaleString()}` : undefined}
        />
        <Mini
          label="热层（内存）"
          value={s ? fmtBytes(s.memory_bytes) : "—"}
          sub={s ? `预算 ${fmtBytes(s.memory_budget_bytes)} · ${s.entries} 条` : undefined}
        />
        <Mini
          label="冷层（磁盘）"
          value={s ? (s.disk_budget_bytes > 0 ? fmtBytes(s.disk_bytes) : "已关闭") : "—"}
          sub={
            s && s.disk_budget_bytes > 0
              ? `硬上限 ${fmtBytes(s.disk_budget_bytes)}（下方「分层预算」设定）`
              : "disk_gb=0 或 persist=false —— 已关闭（可在下方「分层预算」开启）"
          }
        />
      </div>

      {/* 全局开关 */}
      <Card className="flex items-center gap-3">
        <Toggle
          checked={s?.enabled_globally ?? false}
          onChange={(v) =>
            setPolicy.mutate({
              scope: "global",
              policy: { ...(globalRow?.policy ?? {}), enabled: v },
            })
          }
        />
        <div className="text-sm">响应缓存全局开关</div>
        <div className="ml-auto">
          <Button variant="danger" onClick={() => setConfirmPurgeAll(true)}>
            清空全部缓存
          </Button>
        </div>
      </Card>

      {/* 分层预算（settings 覆盖 config.yaml，下次网关重启生效）——逻辑在 cache/TierCard。 */}
      <TierCard onSaved={() => stats.refetch()} />

      {/* 按模型策略矩阵 */}
      <Card>
        <div className="mb-3 flex items-center justify-between">
          <h2 className="text-sm font-semibold">按模型策略</h2>
          <div className="flex items-center gap-2">
            <select
              value={newModel}
              onChange={(e) => setNewModel(e.target.value)}
              className={fieldCls + " w-56"}
            >
              <option value="">
                {candidateModels.length === 0
                  ? modelMeta.isLoading
                    ? "加载模型中…"
                    : "无可选模型（均已添加或后端未上报）"
                  : "选择模型…"}
              </option>
              {candidateModels.map((m) => (
                <option key={m} value={m}>
                  {m}
                </option>
              ))}
            </select>
            <Button
              variant="primary"
              disabled={!newModel}
              onClick={() => {
                setPolicy.mutate({
                  scope: `model:${newModel}`,
                  policy: { enabled: true, ttl: 3600, key_granularity: "strict" },
                });
                setNewModel("");
              }}
            >
              添加模型
            </Button>
          </div>
        </div>
        {/* 读取失败 ≠ 尚无策略：红字 + 重试 */}
        {policies.isError ? (
          <QueryError title="读取缓存策略失败" error={policies.error} onRetry={() => policies.refetch()} />
        ) : modelRows.length === 0 ? (
          <p className="text-xs text-ink-4">
            尚无按模型策略。添加后可就地改 TTL / 键粒度，或删除恢复跟随全局。
          </p>
        ) : (
          <table className="w-full text-xs">
            <thead className={theadCls}>
              <tr className="border-b border-slate-800 text-left text-ink-4">
                <th className="py-1.5 font-medium">模型</th>
                <th className="font-medium">启用</th>
                <th className="font-medium">TTL（秒）</th>
                <th className="font-medium">键粒度</th>
                <th className="text-right font-medium">操作</th>
              </tr>
            </thead>
            <tbody>
              {modelRows.map((row) => {
                const model = row.scope.slice("model:".length);
                return (
                  <tr key={row.scope} className="border-b border-slate-800/60">
                    <td className="py-1.5 font-mono text-ink-2">{model}</td>
                    <td>
                      <Toggle
                        checked={row.policy.enabled ?? false}
                        onChange={(v) =>
                          setPolicy.mutate({ scope: row.scope, policy: { ...row.policy, enabled: v } })
                        }
                      />
                    </td>
                    <td>
                      <input
                        type="number"
                        min={0}
                        value={row.policy.ttl ?? 3600}
                        onChange={(e) =>
                          setPolicy.mutate({
                            scope: row.scope,
                            policy: { ...row.policy, ttl: Number(e.target.value) },
                          })
                        }
                        className="w-24 rounded border border-slate-700 bg-slate-800/60 px-1.5 py-0.5 text-xs"
                      />
                    </td>
                    <td>
                      <select
                        value={row.policy.key_granularity ?? "strict"}
                        onChange={(e) =>
                          setPolicy.mutate({
                            scope: row.scope,
                            policy: { ...row.policy, key_granularity: e.target.value },
                          })
                        }
                        className="rounded border border-slate-700 bg-slate-800/60 px-1.5 py-0.5 text-xs"
                      >
                        <option value="strict">strict（含参数）</option>
                        <option value="loose">loose（仅对话）</option>
                      </select>
                    </td>
                    <td className="text-right">
                      <span className="mr-2 text-ink-4">
                        <Button onClick={() => purge.mutate(model)}>清条目</Button>{" "}
                        <Button variant="danger" onClick={() => setConfirmDelPolicy(row.scope)}>
                          删除
                        </Button>
                      </span>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </Card>

      {confirmPurgeAll && (
        <ConfirmDialog
          open
          title="清空全部响应缓存？"
          onClose={() => setConfirmPurgeAll(false)}
          onConfirm={() => purge.mutate(undefined)}
          confirmLabel="确认清空"
          danger
        >
          <Risk>已缓存的全部响应会被永久删除</Risk>，之后的请求一律回源重算，
          命中率收益清零且无法找回。想只清某个模型，请用下方各行的「清条目」。
        </ConfirmDialog>
      )}

      {confirmDelPolicy && (
        <ConfirmDialog
          open
          title="删除该模型的缓存策略？"
          onClose={() => setConfirmDelPolicy(null)}
          onConfirm={() => delPolicy.mutate(confirmDelPolicy)}
          confirmLabel="确认删除"
        >
          删除后该模型<b>回落到全局策略</b>（当前为「{globalRow ? "已设" : "未设"}」），
          已缓存的条目不受影响。
        </ConfirmDialog>
      )}
    </div>
  );
}

function Mini({ label, value, sub }: { label: string; value: string; sub?: string }) {
  return (
    <Card>
      <div className="text-xs text-ink-4">{label}</div>
      <div className="mt-1 text-xl font-semibold text-ink-1">{value}</div>
      {sub && <div className="mt-1 text-xs text-ink-4">{sub}</div>}
    </Card>
  );
}
