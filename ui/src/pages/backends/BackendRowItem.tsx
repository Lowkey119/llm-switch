// 单个后端行（计划 §6.1 自 Backends.tsx 拆出，行为不变）：
// chevron + 健康点 + 名称 + url + 掩码 key，url/key 原位编辑，整行点击展开模型清单。
import { useToast } from "../../components/Toast";
import { IconButton, StatusDot, Toggle } from "../../components/ui";
import { DivRow } from "../../components/Row";
import { InlineEdit } from "../../components/InlineEdit";
import { ModelRows } from "../../components/ModelRows";
import {
  IconChevron,
  IconConsole,
  IconLock,
  IconPencil,
  IconRefresh,
  IconTrash,
} from "../../components/icons";
import { isProtectedHalogen } from "../../lib/backend";
import { gotoConsole } from "../../lib/nav";
import type { ParamRow } from "../../lib/params";
import type { BackendRow, MatrixCell, MatrixClient, ModelMetric } from "../../lib/ipc";

/** 单行后端：chevron + 健康点 + 名称 + url + 掩码 key；url/key 原位编辑；整行点击展开。 */
export function BackendRowItem({
  b,
  busy,
  expanded,
  onToggle,
  onRefresh,
  refreshing,
  onEdit,
  onEnabled,
  onSaveField,
  paramRows,
  metrics,
  cells,
  clients,
  occ,
  onOpenParams,
  onChanged,
  onDelete,
}: {
  b: BackendRow;
  busy: boolean;
  expanded: boolean;
  onToggle: () => void;
  onRefresh: () => void;
  refreshing: boolean;
  onEdit: () => void;
  onEnabled: (v: boolean) => void;
  onSaveField: (field: "url" | "key", value: string) => void;
  paramRows: ParamRow[];
  metrics: Record<string, ModelMetric>;
  cells: Record<string, Record<string, MatrixCell>>;
  clients: MatrixClient[];
  occ: Record<string, Record<string, string>>;
  onOpenParams: (model: string) => void;
  onChanged: () => void;
  onDelete: () => void;
}) {
  const toast = useToast();

  return (
    <div className="group/row">
      <DivRow expanded={expanded} onClick={onToggle} className="flex items-center gap-3 px-4 py-2.5 hover:bg-slate-800/30">
        <IconChevron
          width={14}
          height={14}
          className={`shrink-0 text-ink-4 transition-transform ${expanded ? "rotate-90" : ""}`}
        />
        <StatusDot state={b.health_state} busy={busy} />
        <span className="shrink-0 truncate text-sm font-medium text-ink-1" title={b.base_url}>
          {b.name}
        </span>
        <InlineEdit
          value={b.base_url}
          className="min-w-0 flex-1 font-mono text-xs text-ink-4"
          inputClassName="min-w-0 flex-1 font-mono"
          title={`${b.base_url}（单击修改，Enter 保存 / Esc 放弃）`}
          ariaLabel="后端 Base URL"
          validate={(v) => {
            if (!/^https?:\/\/.+\S/.test(v)) {
              toast.err("URL 无效，未修改");
              return "invalid";
            }
            return null;
          }}
          onSave={(v) => onSaveField("url", v)}
        />
        {b.api_key && (
          <InlineEdit
            value=""
            display="***"
            placeholder="sk-…"
            inputClassName="w-[130px] font-mono"
            title="API Key（单击覆盖，留空或 Esc 放弃）"
            ariaLabel="API Key"
            validate={(v) => {
              if (v === "***") {
                toast.err("星号是占位符，请输入真实密钥");
                return "mask";
              }
              return null;
            }}
            onSave={(v) => onSaveField("key", v)}
          />
        )}
        <div className="flex shrink-0 items-center gap-2" onClick={(e) => e.stopPropagation()}>
          {/* 启用开关折叠时也要可见：放在行右侧动作区最前 */}
          <span title={b.enabled ? "点击停用" : "点击启用"}>
            <Toggle checked={b.enabled} onChange={onEnabled} />
          </span>
          <IconButton title="刷新：探活 + 重扫模型列表（需确认）" onClick={onRefresh}>
            <span className={refreshing ? "animate-spin inline-flex" : "inline-flex"}>
              <IconRefresh width={14} height={14} />
            </span>
          </IconButton>
          <IconButton title="编辑" onClick={onEdit}>
            <IconPencil width={14} height={14} />
          </IconButton>
          <IconButton
            title="控制台"
            onClick={() => gotoConsole(b.name)}
          >
            <IconConsole width={14} height={14} />
          </IconButton>
          {isProtectedHalogen(b) ? (
            /* halogen 是速率标定与 halogen/gufo 控制台的锚点，后端一律拒删。
               留一枚锁而不是灰掉的垃圾桶：点它照样出说明，用户知道为什么而不是撞墙。 */
            <IconButton title="halogen 后端受保护，不能删除（点我看说明）" onClick={onDelete}>
              <IconLock width={14} height={14} />
            </IconButton>
          ) : (
            <IconButton title="删除后端（需确认）" tone="danger" onClick={onDelete}>
              <IconTrash width={14} height={14} />
            </IconButton>
          )}
        </div>
      </DivRow>

      {b.last_error && (
        <p className="truncate px-4 pb-2 pl-[var(--indent-row)] text-xs text-[var(--sem-danger-text)]" title={b.last_error}>
          最近错误：{b.last_error}
        </p>
      )}

      {expanded && (
        <div className="space-y-3 px-4 pb-4 pl-[var(--indent-row)]">
          <div className="flex items-center gap-3 text-xs text-ink-4">
            <span className="truncate font-mono" title={b.base_url}>{b.base_url}</span>
          </div>
          {b.models.length === 0 ? (
            <p className="text-xs text-ink-4">没有模型</p>
          ) : (
            <ModelRows
              b={b}
              paramRows={paramRows}
              metrics={metrics}
              cells={cells}
              clients={clients}
              occ={occ}
              onOpenParams={onOpenParams}
              onChanged={onChanged}
            />
          )}
        </div>
      )}
    </div>
  );
}

