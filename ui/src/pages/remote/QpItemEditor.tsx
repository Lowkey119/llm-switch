// 快速面板档的条目原位编辑（自 QuickPanelBoard 拆出，行为一字未改）。
// 拆出去是因为主文件过了 ui-budget 的 550 行线——法条是「超预算先拆，不往里塞」。
import { useRef } from "react";
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
}: {
  item: QpItem;
  onCommit: (it: QpItem) => void;
  onClose: () => void;
}) {
  const kind = itemKind(item);
  // 输入法组字门控（计划 2026-11-26「中文输入法拼音混入 bug」）：受控 input 在 composition
  // 组字期间就提交，会把拼音字母写进条目（用户实报「发布」存成 `wwq你  dvv`）。
  // 组字期间不提交，composition 结束再提交最终值。StepEditor 用本地 state 无此问题。
  const composing = useRef(false);
  const guard = (commit: (v: string) => void) => ({
    onChange: (e: React.ChangeEvent<HTMLInputElement>) => {
      if (!composing.current) commit(e.target.value);
    },
    onCompositionStart: () => {
      composing.current = true;
    },
    onCompositionEnd: (e: React.CompositionEvent<HTMLInputElement>) => {
      composing.current = false;
      commit(e.currentTarget.value);
    },
  });
  if (kind === "action") {
    if (item.action!.type === "shortcut" && !isPresetShortcut(item.action!.shortcut)) {
      // 非预设快捷键：行内没有可编辑字段，改写由调用方在事件里路由回底栏键盘（openEdit）。
      // 这里**绝不能在渲染期回调 onRewrite**：那是渲染期 setState，会构成
      // QPB→QpItemList→ItemEditor 无限渲染环（fix:virtual-keyboard-freeze-recurrence 真凶，
      // dev 报 Maximum update depth、正式包 WebContent 吃满一核）。
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
        {...guard((v) => onCommit({ cmd: v, t: item.t }))}
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
    <input
      autoFocus
      className={`${fieldCls} min-w-0 flex-1`}
      aria-label={t("remote.qp.itemText")}
      value={item.t ?? ""}
      {...guard((v) => onCommit({ t: v }))}
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
