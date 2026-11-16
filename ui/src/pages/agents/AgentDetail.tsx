// 右侧「选中代理详情」：接管动作、配置文件路径、Codex wire_api、模型分配、扩展、备份。
//
// 各节自带的 query/mutation 都挂在组件内，随节挂载卸载 —— 之前全挤在 AgentsPage 里，
// 页面层要背七八个 state，其实没有第二处用它们。
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useToast } from "../../components/Toast";
import { Badge, Button, Card } from "../../components/ui";
import { QueryError } from "../../components/QueryError";
import { ClientModelPicker } from "../../components/ClientModelPicker";
import { ipc, type MatrixClient } from "../../lib/ipc";
import { ExtSection, EXT_CLIENTS, RateLineSection } from "./ExtSection";
import { PathList } from "./PathList";
import { BackupSection } from "./BackupSection";
import { useTakeoverAct } from "./useTakeoverAct";
import { when, type ClientRow, type TakeoverStatus } from "./types";

interface Props {
  row: ClientRow;
  /** 列源里的清单条目（槽位/档位口径），停用的代理取不到。 */
  meta?: MatrixClient;
  status: TakeoverStatus;
  isOff: boolean;
  onToggleCfg: () => void;
  /** 打开配置面板的按钮本体：面板 Esc/✕ 关闭后焦点归还这里（键盘可达）。 */
  cfgTriggerRef?: React.Ref<HTMLButtonElement>;
}

export function AgentDetail({ row, meta, status, isOff, onToggleCfg, cfgTriggerRef }: Props) {
  const qc = useQueryClient();
  const toast = useToast();
  const act = useTakeoverAct();

  const settings = useQuery({
    queryKey: ["takeover-settings"],
    queryFn: () => ipc<{ codex_wire_api: string }>("get_takeover_settings"),
  });
  const wire = useMutation({
    mutationFn: (value: string) => ipc("set_takeover_settings", { value }),
    onSuccess: () => {
      toast.ok("Codex wire_api 已更新（下次接管生效）");
      qc.invalidateQueries({ queryKey: ["takeover-settings"] });
      qc.invalidateQueries({ queryKey: ["takeover"] });
    },
    onError: (e) => toast.err(`操作失败：${e}`),
  });
  // 代理启停（片三）：只动清单——不碰配置文件、不清分配。
  const toggleAgent = useMutation({
    mutationFn: (v: { app: string; enabled: boolean }) => ipc("agents_set_enabled", v),
    onSuccess: (_r, v) => {
      toast.ok(
        v.enabled
          ? "已启用：列表与两张矩阵恢复显示（分配原样）"
          : "已停用：仅从列表与两张矩阵隐藏（接管与分配保留）",
      );
      qc.invalidateQueries({ queryKey: ["client-list"] });
      qc.invalidateQueries({ queryKey: ["matrix"] });
      qc.invalidateQueries({ queryKey: ["takeover"] });
    },
    onError: (e) => toast.err(`操作失败：${e}`),
  });
  const busy = act.isPending || wire.isPending;

  return (
    <Card>
      <div className="flex flex-wrap items-center gap-2">
        <h2 className="text-sm font-semibold">{row.label}</h2>
        {row.error ? (
          <Badge>读取失败</Badge>
        ) : row.managed ? (
          <Badge>已接管</Badge>
        ) : row.pending ? (
          <Badge>待接管</Badge>
        ) : (
          <Badge>—</Badge>
        )}
        {isOff && (
          <span className="rounded border border-amber-500/40 bg-amber-500/10 px-1.5 py-0.5 text-xs text-[var(--sem-busy-text)]">
            已停用
          </span>
        )}
        {meta && meta.slots.length > 0 && (
          <span className="rounded border border-slate-700 px-1.5 py-0.5 text-2xs text-ink-3">
            {meta.tiers.length > 0
              ? `多档：main + ${meta.tiers.join(" / ")}`
              : "双槽：主模型 / 小模型"}
          </span>
        )}
        {status.endpoint && (
          <span className="ml-auto font-mono text-xs text-ink-4">
            {status.endpoint.base_root} · {status.endpoint.models.length} 个模型
          </span>
        )}
      </div>
      {isOff && row.managed && (
        <p className="mt-1 text-xs leading-relaxed text-[var(--sem-busy-text)]">
          该代理已停用但仍处接管状态：配置文件保持接管时的写入、未回滚；重新启用即恢复原样。
        </p>
      )}

      <div className="mt-2 flex flex-wrap items-center gap-2 text-xs text-ink-4">
        <span>
          上次接管 {when(row.applied_at)}
          {row.reverted_at && ` · 回滚 ${when(row.reverted_at)}`}
        </span>
        <span className="ml-auto flex gap-2">
          <Button
            variant="ghost"
            disabled={busy || !!row.error || !row.pending}
            onClick={() => act.mutate({ cmd: "takeover_apply", args: { app: row.app } })}
          >
            接管
          </Button>
          <Button
            variant="ghost"
            disabled={busy || !row.latest_backup}
            onClick={() => act.mutate({ cmd: "takeover_revert", args: { app: row.app } })}
          >
            回滚
          </Button>
          <Button variant="ghost" disabled={!!row.error} onClick={onToggleCfg} ref={cfgTriggerRef}>
            配置文件
          </Button>
          <Button
            variant="ghost"
            disabled={toggleAgent.isPending}
            title={
              isOff
                ? "重新启用：恢复列表与两张矩阵的显示（分配原样保留）"
                : "停用：仅从列表与两张矩阵隐藏——不碰配置文件、不清分配"
            }
            onClick={() => toggleAgent.mutate({ app: row.app, enabled: !isOff })}
          >
            {isOff ? "启用" : "停用"}
          </Button>
        </span>
      </div>

      {row.error ? (
        <div className="mt-2 break-all text-xs text-[var(--sem-busy-text)]">{row.error}</div>
      ) : (
        <>
          <PathList app={row.app} files={row.files} home={status.home} />
          {row.app === "codex" && settings.isError && (
            <div className="mt-2">
              <QueryError inline title="读取 wire_api 设置失败" error={settings.error} onRetry={() => settings.refetch()} />
            </div>
          )}
          {row.app === "codex" && settings.data && (
            <label className="mt-2 flex items-center gap-2 text-xs text-ink-4">
              Codex wire_api
              <select
                value={settings.data.codex_wire_api}
                disabled={wire.isPending}
                onChange={(e) => wire.mutate(e.target.value)}
                className="rounded-md border border-slate-700 bg-slate-900 px-2 py-1 text-xs"
              >
                <option value="chat">chat（/v1/chat/completions）</option>
                <option value="responses">responses（/v1/responses）</option>
              </select>
            </label>
          )}
          {/* 模型分配：默认档 + 各档位（多档代理），真值来自配置文件 */}
          <div className="mt-3 border-t border-slate-800 pt-2">
            <div className="mb-1 flex items-center justify-between">
              <span className="text-xs font-medium text-ink-2">模型分配</span>
            </div>
            <ClientModelPicker
              app={row.app}
              current={row.model.current}
              managed={row.managed}
              note={row.model.note}
            />
          </div>
        </>
      )}

      {/* 速率扩展（一键安装/卸载）：八家各按自己的官方机制装，数值与浮窗速率线同源。 */}
      {EXT_CLIENTS[row.app] && <ExtSection app={row.app} />}

      {/* 速率状态线：所有代理共用 llm-switch 浮窗的单行模式，不再各家写插件。 */}
      <RateLineSection app={row.app} />

      <BackupSection app={row.app} />
    </Card>
  );
}
