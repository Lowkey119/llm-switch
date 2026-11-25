// 单个后端行（计划 §6.1 自 Backends.tsx 拆出，行为不变）：
// chevron + 健康点 + 名称 + url + 掩码 key，url/key 原位编辑，整行点击展开模型清单。
import { useToast } from "../../components/Toast";
import { t, ts } from "../../lib/i18n";
import { IconButton, StatusDot } from "../../components/ui";
import { DirectModeControl } from "../../components/DirectModeControl";
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
  managedCount,
  vlmCount,
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
  /** 已接管代理数（直连确认框/toast 的 N）。 */
  managedCount: number;
  /** 视觉模型数（行头「视觉·N」徽标，判定口径与展开区同源）。 */
  vlmCount: number;
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
        {/* 视觉·N 徽标上移到身份区（用户草图口径）：计数与展开区分组同源（vlm 种类判定）。 */}
        {vlmCount > 0 && (
          <span className="shrink-0 rounded bg-slate-700/60 px-1.5 py-0.5 text-2xs text-ink-2">
            {ts("row.vlmBadge", { n: vlmCount })}
          </span>
        )}
        <InlineEdit
          value={b.base_url}
          className="min-w-0 flex-1 font-mono text-xs text-ink-4"
          inputClassName="min-w-0 flex-1 font-mono"
          title={ts("bri.editTip", { url: b.base_url })}
          ariaLabel={t("row.urlAria")}
          validate={(v) => {
            if (!/^https?:\/\/.+\S/.test(v)) {
              toast.err(t("row.urlInvalid"));
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
            title={t("row.keyTitle")}
            ariaLabel="API Key"
            validate={(v) => {
              if (v === "***") {
                toast.err(t("row.keyPlaceholderOnly"));
                return "mask";
              }
              return null;
            }}
            onSave={(v) => onSaveField("key", v)}
          />
        )}
        {/* 三段式两端对齐（计划排版节）：身份区（名称/地址/Key）— 弹性空隙 — 指标+动作簇 — 固定间距 — 三态控件。
            后端行级指标 = 模型数（tabular-nums 纵向对齐）；健康状态由 StatusDot 自带文字，不重复。 */}
        <span className="hidden shrink-0 items-center gap-3 text-sm tabular-nums text-ink-4 lg:flex">
          {b.models.length > 0 && <span>{ts("row.modelCount", { n: b.models.length })}</span>}
        </span>
        <div className="ml-6 flex shrink-0 items-center gap-2" onClick={(e) => e.stopPropagation()}>
          <IconButton
            title={b.enabled ? t("row.refreshTitle") : t("row.refreshDisabledTitle")}
            disabled={!b.enabled}
            onClick={onRefresh}
          >
            <span className={refreshing ? "animate-spin inline-flex" : "inline-flex"}>
              <IconRefresh width={14} height={14} />
            </span>
          </IconButton>
          <IconButton title={t("row.edit")} onClick={onEdit}>
            <IconPencil width={14} height={14} />
          </IconButton>
          <IconButton
            title={t("row.console")}
            onClick={() => gotoConsole(b.name)}
          >
            <IconConsole width={14} height={14} />
          </IconButton>
          {isProtectedHalogen(b) ? (
            /* halogen 是速率标定与 halogen/gufo 控制台的锚点，后端一律拒删。
               留一枚锁而不是灰掉的垃圾桶：点它照样出说明，用户知道为什么而不是撞墙。 */
            <IconButton title={t("row.protectedTitle")} onClick={onDelete}>
              <IconLock width={14} height={14} />
            </IconButton>
          ) : (
            <IconButton title={t("row.deleteTitle")} tone="danger" onClick={onDelete}>
              <IconTrash width={14} height={14} />
            </IconButton>
          )}
        </div>
        {/* 取用三态控件钉在最右（计划排版节：动作区之后固定间距）；折叠时也要可见。 */}
        <div className="ml-6 flex shrink-0 items-center gap-2" onClick={(e) => e.stopPropagation()}>
          <DirectModeControl
            b={b}
            managedCount={managedCount}
            onDisable={() => onEnabled(false)}
            onEnable={() => onEnabled(true)}
            onChanged={onChanged}
          />
        </div>
      </DivRow>

      {b.last_error && (
        <p className="truncate px-4 pb-2 pl-[var(--indent-row)] text-sm text-[var(--sem-danger-text)]" title={b.last_error}>
          最近错误：{b.last_error}
        </p>
      )}

      {expanded && (
        <div className="space-y-3 px-4 pb-4 pl-[var(--indent-row)]">
          {b.models.length === 0 ? (
            <p className="text-sm text-ink-4">{t("row.noModels")}</p>
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

