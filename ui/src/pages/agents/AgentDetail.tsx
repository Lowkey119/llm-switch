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
import { t, ts } from "../../lib/i18n";
import { ExtSection, EXT_CLIENTS, RateLineSection } from "./ExtSection";
import { PonytailSection } from "./PonytailSection";
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
      toast.ok(t("ag.wireUpdated"));
      qc.invalidateQueries({ queryKey: ["takeover-settings"] });
      qc.invalidateQueries({ queryKey: ["takeover"] });
    },
    onError: (e) => toast.err(ts("ag.actFail", { e: String(e) })),
  });
  // 代理启停（片三）：只动清单——不碰配置文件、不清分配。
  const toggleAgent = useMutation({
    mutationFn: (v: { app: string; enabled: boolean }) => ipc("agents_set_enabled", v),
    onSuccess: (_r, v) => {
      toast.ok(v.enabled ? t("ag.enabledOk") : t("ag.disabledOk"));
      qc.invalidateQueries({ queryKey: ["client-list"] });
      qc.invalidateQueries({ queryKey: ["matrix"] });
      qc.invalidateQueries({ queryKey: ["takeover"] });
    },
    onError: (e) => toast.err(ts("ag.actFail", { e: String(e) })),
  });
  const busy = act.isPending || wire.isPending;

  return (
    <Card>
      <div className="flex flex-wrap items-center gap-2">
        <h2 className="text-base font-semibold">{row.label}</h2>
        {row.error ? (
          <Badge>{t("ag.readFailBadge")}</Badge>
        ) : row.managed ? (
          <Badge>{t("ag.takenOver")}</Badge>
        ) : row.pending ? (
          <Badge>{t("ag.pendingTakeover")}</Badge>
        ) : (
          <Badge>—</Badge>
        )}
        {isOff && (
          <span className="rounded border border-amber-500/40 bg-amber-500/10 px-1.5 py-0.5 text-sm text-[var(--sem-busy-text)]">
            {t("ag.disabled")}
          </span>
        )}
        {meta && meta.slots.length > 0 && (
          <span className="rounded border border-slate-700 px-1.5 py-0.5 text-2xs text-ink-3">
            {meta.tiers.length > 0
              ? ts("ag.multiTier", { tiers: meta.tiers.join(" / ") })
              : t("ag.dualSlot")}
          </span>
        )}
        {/* ui_04：动作按钮组上提到标题行右端；endpoint 读数下移到元信息行，不再三层右对齐错位。 */}
        <span className="ml-auto flex gap-2">
          <Button
            variant="ghost"
            /* fix:agent-detail-assign-and-takeover 1 号：已接管就灰显，不再看 pending。
               配置漂移时 managed 与 pending 会并存（`takeover/claude.rs` 的
               「模型漂了必须重新 pending」），那时徽标已写「已接管」而按钮还能点，
               两张皮。按用户口径：漂移后的「重新接管」入口随按钮一起灰掉，
               不再另留第二个入口。 */
            disabled={busy || !!row.error || row.managed}
            onClick={() => act.mutate({ cmd: "takeover_apply", args: { app: row.app } })}
          >
            {t("ag.takeover")}
          </Button>
          <Button
            variant="ghost"
            disabled={busy || !row.latest_backup}
            onClick={() => act.mutate({ cmd: "takeover_revert", args: { app: row.app } })}
          >
            {t("ag.revert")}
          </Button>
          <Button variant="ghost" disabled={!!row.error} onClick={onToggleCfg} ref={cfgTriggerRef}>
            {t("ag.cfgFile")}
          </Button>
          <Button
            variant="ghost"
            disabled={toggleAgent.isPending}
            title={isOff ? t("ag.reEnableTip") : t("ag.disableTip")}
            onClick={() => toggleAgent.mutate({ app: row.app, enabled: !isOff })}
          >
            {isOff ? t("ag.enable") : t("ag.disableBtn")}
          </Button>
        </span>
      </div>
      {isOff && row.managed && (
        <p className="mt-1 text-sm leading-relaxed text-[var(--sem-busy-text)]">
          {t("ag.residualBody")}
        </p>
      )}

      <div className="mt-2 flex flex-wrap items-center gap-2 text-sm text-ink-4">
        <span>
          {ts("ag.lastTakeover", { t: when(row.applied_at) })}
          {row.reverted_at && ts("ag.lastRevert", { t: when(row.reverted_at) })}
        </span>
        {status.endpoint && (
          <span className="font-mono text-sm text-ink-3">
            {status.endpoint.base_root} · {ts("ag.modelsN", { n: status.endpoint.models.length })}
          </span>
        )}
      </div>

      {row.error ? (
        <div className="mt-2 break-all text-sm text-[var(--sem-busy-text)]">{row.error}</div>
      ) : (
        <>
          <PathList app={row.app} files={row.files} home={status.home} />
          {row.app === "codex" && settings.isError && (
            <div className="mt-2">
              <QueryError inline title={t("ag.wireReadFail")} error={settings.error} onRetry={() => settings.refetch()} />
            </div>
          )}
          {row.app === "codex" && settings.data && (
            <label className="mt-2 flex items-center gap-2 text-sm text-ink-4">
              Codex wire_api
              <select
                value={settings.data.codex_wire_api}
                disabled={wire.isPending}
                onChange={(e) => wire.mutate(e.target.value)}
                className="rounded-md border border-slate-700 bg-slate-900 px-2 py-1 text-sm"
              >
                <option value="chat">chat（/v1/chat/completions）</option>
                <option value="responses">responses（/v1/responses）</option>
              </select>
            </label>
          )}
          {/* 模型分配：默认档 + 各档位（多档代理），真值来自配置文件 */}
          <div className="mt-3 border-t border-slate-800 pt-2">
            <div className="mb-1 flex items-center justify-between">
              <span className="text-sm font-medium text-ink-2">{t("ag.assignTitle")}</span>
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

      {/* ponytail 插件（第三方）：与速率扩展同一套机器，只是 pack=ponytail；
          装/卸/查上游都在这一节，逐端机制走各家官方 CLI。 */}
      {EXT_CLIENTS[row.app] && <PonytailSection app={row.app} />}

      {/* 速率状态线：所有代理共用 llm-switch 浮窗的单行模式，不再各家写插件。 */}
      <RateLineSection app={row.app} />

      <BackupSection app={row.app} />
    </Card>
  );
}
