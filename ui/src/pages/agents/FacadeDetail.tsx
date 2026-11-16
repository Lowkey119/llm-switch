// 网关门面（copilot）的详情卡。
//
// 为什么单独一卡：门面**没有可接管的配置文件**（可见清单存 settings 表，`read_selection`
// 对它显式报错），所以常规详情卡里的接管/回滚/配置文件/备份节对它都不成立。但它装的
// CLI 是真装在本机的 —— 装上了就该和普通代理一样有个管理位，而不是只活在矩阵里。
//
// 分工照旧：模型可见性的**写入口仍是矩阵格子**（`takeover::copilot_assign`），这里只读
// 摘要 + 一键跳过去分配；装没装、装哪版、装速率扩展在这里。
import { Badge, Button, Card } from "../../components/ui";
import type { AgentScan, MatrixClient } from "../../lib/ipc";
import { EXT_CLIENTS, ExtSection, RateLineSection } from "./ExtSection";
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
  return (
    <Card>
      <div className="flex flex-wrap items-center gap-2">
        <h2 className="text-sm font-semibold">{meta.label}</h2>
        <Badge>网关门面</Badge>
        {scan?.bin_found ? (
          <span className="rounded border border-emerald-500/40 bg-emerald-500/10 px-1.5 py-0.5 text-2xs text-[var(--sem-ok-text)]">
            已装 {scan.version ?? scan.bin}
          </span>
        ) : (
          <span className="rounded border border-slate-700 bg-slate-800/60 px-1.5 py-0.5 text-2xs text-ink-4">
            未检测到 CLI
          </span>
        )}
        <span className="ml-auto text-xs text-ink-4">
          已可见 {meta.visible.length} 个模型
        </span>
      </div>

      <div className="mt-3 border-t border-slate-800 pt-2">
        <div className="mb-1 flex items-center justify-between">
          <span className="text-xs font-medium text-ink-2">可见模型</span>
          <Button size="sm" variant="ghost" onClick={onAssign}>
            去矩阵分配
          </Button>
        </div>
        {meta.visible.length === 0 ? (
          <p className="text-xs text-ink-4">
            一个都没勾：门面此刻不暴露任何模型，客户端问 <span className="font-mono">/api/tags</span> 会拿到空清单。
          </p>
        ) : (
          <ul className="max-h-40 overflow-auto rounded-md border border-slate-800 bg-slate-900/40 px-3 py-2">
            {meta.visible.map((m) => (
              <li key={m} className="truncate font-mono text-2xs text-ink-3" title={m}>
                {m}
              </li>
            ))}
          </ul>
        )}
      </div>

      {scan && (
        <div className="mt-3 border-t border-slate-800 pt-2">
          <div className="mb-1.5 text-xs font-medium text-ink-2">CLI 安装</div>
          <InstallRow s={scan} onInstalled={onInstalled} />
        </div>
      )}

      {/* 速率扩展：门面也装同一个速率扩展（copilot plugin install），数值与浮窗同源。 */}
      {EXT_CLIENTS[meta.app] && <ExtSection app={meta.app} />}
      <RateLineSection app={meta.app} />
    </Card>
  );
}
