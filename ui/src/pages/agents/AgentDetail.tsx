// 右侧「选中代理详情」：接管动作、配置文件路径、Codex wire_api、模型分配、扩展、备份。
//
// 各节自带的 query/mutation 都挂在组件内，随节挂载卸载 —— 之前全挤在 AgentsPage 里，
// 页面层要背七八个 state，其实没有第二处用它们。
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useToast } from "../../components/Toast";
import { Button, Card } from "../../components/ui";
import { QueryError } from "../../components/QueryError";
import { ClientModelPicker } from "../../components/ClientModelPicker";
import { ipc } from "../../lib/ipc";
import { t, ts } from "../../lib/i18n";
import { ExtSection, EXT_CLIENTS, RateLineSection } from "./ExtSection";
import { PonytailSection } from "./PonytailSection";
import { PathList } from "./PathList";
import { BackupSection } from "./BackupSection";
import { InstallRow } from "./InstallPanel";
import { useTakeoverAct } from "./useTakeoverAct";
import { type AgentScan } from "../../lib/ipc";
import { type ClientRow, type TakeoverStatus } from "./types";

interface Props {
  row: ClientRow;
  status: TakeoverStatus;
  isOff: boolean;
  onToggleCfg: () => void;
  /** 打开配置面板的按钮本体：面板 Esc/✕ 关闭后焦点归还这里（键盘可达）。 */
  cfgTriggerRef?: React.Ref<HTMLButtonElement>;
  /** 本端安装扫描结果（装没装、版本、一键装命令）；冷启动没到货 = 不摆这节。 */
  scan?: AgentScan;
  /** 单端「检测新版本」：强制重探本端可执行与版本。 */
  onCheckUpdate?: () => void;
}

export function AgentDetail({ row, status, isOff, onToggleCfg, cfgTriggerRef, scan, onCheckUpdate }: Props) {
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
        {/* 接管状态徽标与「多档：…」档位徽标已删（用户 2026-11-23）：
            接管与否看左列行尾的「已接管」，档位槽数左列行上也有，详情头不再重复挂。 */}
        {isOff && (
          <span className="rounded border border-amber-500/40 bg-amber-500/10 px-1.5 py-0.5 text-sm text-[var(--sem-busy-text)]">
            {t("ag.disabled")}
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
      {/* 「上次接管 … 网关地址 · N 个模型」元信息行已删（用户 2026-11-23）：
          接管时间不是日常读数，网关地址与模型数顶栏胶囊本来就有。 */}

      {/* CLI 安装节（原「安装」tab 的内容并入各代理详情，用户 2026-11-23）：
          装没装、版本、一键装/重装、复制命令、配置落点，都在自家入口管。
          「检测新版本」强制重探本端可执行与版本（后台飞，到货自动更新读数）。 */}
      {scan && (
        <div className="mt-2">
          <div className="mb-1 flex items-center justify-between gap-2">
            <span className="text-sm font-medium text-ink-2">{t("ag.cliInstall")}</span>
            {onCheckUpdate && (
              <Button size="xs" variant="ghost" onClick={onCheckUpdate}>
                {t("ag.checkUpdate")}
              </Button>
            )}
          </div>
          <InstallRow s={scan} onInstalled={onCheckUpdate ?? (() => {})} />
        </div>
      )}

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

      {/* 速率扩展 + ponytail 插件并排两张卡（用户 2026-11-23）：两节同一套后端机器
          （pack 分家），并列后各自成框；窄容器 cards-2 自动退成一列。 */}
      {EXT_CLIENTS[row.app] && (
        <div className="cards-2 mt-3 gap-3">
          <ExtSection app={row.app} framed />
          <PonytailSection app={row.app} framed />
        </div>
      )}

      {/* 速率状态线：所有代理共用 llm-switch 浮窗的单行模式，不再各家写插件。 */}
      <RateLineSection app={row.app} />

      <BackupSection app={row.app} />
    </Card>
  );
}
