// 通用二次确认弹窗（UI 重设计）：刷新模型列表、更换转发端口等破坏性动作的闸门。
// 复用 Radix Dialog（与 BackendDialog 同族，零新依赖）；文案里说清「会发生什么」。
import * as Dialog from "@radix-ui/react-dialog";
import type { ReactNode } from "react";
import { Button } from "./ui";
import { IconAlert } from "./icons";
import { t } from "../lib/i18n";

/**
 * 后果句专用（UI-AUDIT P0-6 / 计划 1.5）。
 *
 * 以前整段说明里的每个 <b> 都被染成危险红，于是「刷新模型列表」这种中性弹窗里的
 * 强调词也一片红，真正的危险反而被稀释。现在 <b> 只是中性强调，
 * 会造成损失的那句话才用 <Risk>。
 */
export function Risk({ children }: { children: ReactNode }) {
  return <span className="text-[var(--sem-danger-text)]">{children}</span>;
}

export function ConfirmDialog({
  open,
  title,
  onClose,
  onConfirm,
  confirmLabel = t("common.confirm"),
  danger = false,
  irreversible = false,
  children,
}: {
  open: boolean;
  /** 点对象：说清动的是哪个东西（「删除后端 xxx」，不是「确认删除」）。 */
  title: string;
  onClose: () => void;
  onConfirm: () => void;
  confirmLabel?: string;
  danger?: boolean;
  /**
   * 不可逆单独标注（U7）。
   *
   * 此前「能不能撤回」散在各自的后果句里，有的写在句中、有的干脆没写，
   * 同一个闸门里最该一眼看到的那条反而最不好找。设了这个就在正文下面单独钉一条，
   * 正文照旧说具体后果（丢了什么、要重做什么）——两者不互相替代。
   */
  irreversible?: boolean;
  /** 后果说明（渲染在正文区，允许 <b> 中性强调与 <Risk> 危险句）。 */
  children: React.ReactNode;
}) {
  return (
    <Dialog.Root open={open} onOpenChange={(o) => !o && onClose()}>
      {/* z-[90] 与命令面板同档：Radix Portal 默认无 z-index，会被遥控器页实物图（z-10）/sticky 卡头穿透。 */}
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-[90] bg-black/60" />
        <Dialog.Content className="fixed left-1/2 top-1/2 z-[90] w-[400px] max-w-[92vw] -translate-x-1/2 -translate-y-1/2 rounded-xl border border-slate-700 bg-slate-900 p-5 shadow-2xl">
          <Dialog.Title className="text-base font-semibold">{title}</Dialog.Title>
          <Dialog.Description className="mt-3 text-sm leading-relaxed text-ink-3 [&_b]:font-medium [&_b]:text-ink-2">
            {children}
          </Dialog.Description>
          {irreversible && (
            <p className="mt-3 flex items-center gap-1.5 rounded border border-rose-500/30 bg-rose-500/10 px-2 py-1.5 text-2xs text-[var(--sem-danger-text)]">
              <IconAlert width={12} height={12} className="shrink-0" aria-hidden="true" />
              {t("dlg.irreversible")}
            </p>
          )}
          <div className="mt-5 flex justify-end gap-2">
            <Button onClick={onClose}>{t("dlg.cancel")}</Button>
            <Button
              variant={danger ? "danger" : "primary"}
              onClick={() => {
                onClose();
                onConfirm();
              }}
            >
              {confirmLabel}
            </Button>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
