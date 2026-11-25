// 网关门面（copilot）的详情卡。
//
// 为什么单独一卡：门面**没有可接管的配置文件**（可见清单存 settings 表，`read_selection`
// 对它显式报错），所以常规详情卡里的接管/回滚/配置文件/备份节对它都不成立。但它装的
// CLI 是真装在本机的 —— 装上了就该和普通代理一样有个管理位，而不是只活在矩阵里。
//
// 分工照旧：模型可见性的**写入口仍是矩阵格子**（`takeover::copilot_assign`），这里只读
// 摘要 + 一键跳过去分配；装没装、装哪版、装速率扩展在这里。
import { Badge, Button, Card } from "../../components/ui";
import { t, tf, ts } from "../../lib/i18n";
import type { AgentScan, MatrixClient } from "../../lib/ipc";
import { groupModels, useModelGroups } from "../../lib/modelGroups";
import { EXT_CLIENTS, ExtSection, RateLineSection } from "./ExtSection";
import { PonytailSection } from "./PonytailSection";
import { InstallRow } from "./InstallPanel";

interface Props {
  meta: MatrixClient;
  /** 该端的安装扫描结果（未扫到 = undefined，节自己会缺省显示）。 */
  scan?: AgentScan;
  /** 跳到「模型分配全景」——可见清单的唯一写入口在那边。 */
  onAssign: () => void;
  onInstalled: () => void;
}

export function FacadeDetail({ meta, scan, onAssign, onInstalled }: Props) {
  // 可见模型按提供它的后端分组（全站同一口径）。组里没有的模型归「其它」，
  // 绝不因为要分组就少列几个——可见清单永远以 meta.visible 为准。
  const modelGroups = useModelGroups();
  const groups = groupModels(modelGroups.data ?? [], meta.visible);
  return (
    <Card>
      <div className="flex flex-wrap items-center gap-2">
        <h2 className="text-base font-semibold">{meta.label}</h2>
        <Badge>{t("ag.gatewayFacade")}</Badge>
        {scan?.bin_found ? (
          <span className="rounded border border-emerald-500/40 bg-emerald-500/10 px-1.5 py-0.5 text-2xs text-[var(--sem-ok-text)]">
            {ts("ag.installedV", { v: scan.version ?? scan.bin })}
          </span>
        ) : (
          <span className="rounded border border-slate-700 bg-slate-800/60 px-1.5 py-0.5 text-2xs text-ink-4">
            {t("ag.noCli")}
          </span>
        )}
        <span className="ml-auto text-sm text-ink-4">
          {ts("ag.visibleModelsN", { n: meta.visible.length })}
        </span>
      </div>

      <div className="mt-3 border-t border-slate-800 pt-2">
        <div className="mb-1 flex items-center justify-between">
          <span className="text-sm font-medium text-ink-2">{t("ag.visibleModels")}</span>
          <Button size="sm" variant="ghost" onClick={onAssign}>
            {t("ag.goMatrix")}
          </Button>
        </div>
        {meta.visible.length === 0 ? (
          <p className="text-sm text-ink-4">
            {tf("ag.noneChecked", { api: <span className="font-mono">/api/tags</span> })}
          </p>
        ) : (
          <div className="max-h-40 overflow-auto rounded-md border border-slate-800 bg-slate-900/40 px-3 py-2">
            {groups.map((g) => (
              <div key={g.backend} className="mb-1.5 last:mb-0">
                <div className="text-2xs text-ink-4">{g.backend}</div>
                <ul>
                  {g.models.map((m) => (
                    <li key={m} className="truncate font-mono text-2xs text-ink-3" title={m}>
                      {m}
                    </li>
                  ))}
                </ul>
              </div>
            ))}
          </div>
        )}
      </div>

      {scan && (
        <div className="mt-3 border-t border-slate-800 pt-2">
          <div className="mb-1.5 text-sm font-medium text-ink-2">{t("ag.cliInstall")}</div>
          <InstallRow s={scan} onInstalled={onInstalled} />
        </div>
      )}

      {/* 速率扩展：门面也装同一个速率扩展（copilot plugin install），数值与浮窗同源。 */}
      {EXT_CLIENTS[meta.app] && <ExtSection app={meta.app} />}
      {/* ponytail：门面同样走自家官方插件机制（copilot plugin install）。 */}
      {EXT_CLIENTS[meta.app] && <PonytailSection app={meta.app} />}
      <RateLineSection app={meta.app} />
    </Card>
  );
}
