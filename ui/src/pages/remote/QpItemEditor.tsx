// 快速面板档的条目原位编辑（自 QuickPanelBoard 拆出，行为一字未改）。
// 拆出去是因为主文件过了 ui-budget 的 550 行线——法条是「超预算先拆，不往里塞」。
import { fieldCls } from "../../components/ui";
import { t } from "../../lib/i18n";
import { StepEditor, isPresetShortcut } from "./StepEditor";
import { itemKind, QP_READONLY, type QpItem } from "./quickpanel";

export function qpNote(type: string) {
  return QP_READONLY.includes(type) ? ` · ${t("remote.qp.roTag")}` : "";
}

/**
 * 条目原位编辑（与步骤编辑器同一分工）：文本/命令走行内输入，单动作交回 `StepEditor`，
 * 普通快捷键由调用方弹底栏键盘（这里不收 shortcut，StepEditor 收到非预设快捷键会返回 null）。
 */
export function ItemEditor({
  item,
  onCommit,
  onClose,
  onRewrite,
}: {
  item: QpItem;
  onCommit: (it: QpItem) => void;
  onClose: () => void;
  /** 普通快捷键条目：把改写交给底栏那把键盘。 */
  onRewrite: () => void;
}) {
  const kind = itemKind(item);
  if (kind === "action") {
    if (item.action!.type === "shortcut" && !isPresetShortcut(item.action!.shortcut)) {
      onRewrite();
      return null;
    }
    return (
      <StepEditor
        step={{ type: "action", action: item.action! }}
        onCommit={(s) => s.type === "action" && onCommit({ action: s.action })}
        onClose={onClose}
      />
    );
  }
  if (kind === "cmd") {
    return (
      <input
        autoFocus
        className={`${fieldCls} min-w-0 flex-1`}
        aria-label={t("remote.qp.itemCmd")}
        value={item.cmd ?? ""}
        onChange={(e) => onCommit({ cmd: e.target.value, t: item.t })}
        onBlur={onClose}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === "Escape") {
            e.stopPropagation();
            onClose();
          }
        }}
      />
    );
  }
  return (
    <div className="flex min-w-0 flex-1 items-center gap-1.5">
      <input
        autoFocus
        className={`${fieldCls} min-w-0 flex-1`}
        aria-label={t("remote.qp.itemText")}
        value={item.t ?? ""}
        onChange={(e) => onCommit({ t: e.target.value, n: item.n })}
        onBlur={onClose}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === "Escape") {
            e.stopPropagation();
            onClose();
          }
        }}
      />
      <input
        className={`${fieldCls} w-28 shrink-0`}
        aria-label={t("remote.qp.itemNote")}
        placeholder={t("remote.qp.itemNote")}
        value={item.n ?? ""}
        onChange={(e) => onCommit({ t: item.t, n: e.target.value })}
        onBlur={onClose}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === "Escape") {
            e.stopPropagation();
            onClose();
          }
        }}
      />
    </div>
  );
}
