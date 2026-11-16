// 通用二次确认弹窗（UI 重设计）：刷新模型列表、更换转发端口等破坏性动作的闸门。
// 复用 Radix Dialog（与 BackendDialog 同族，零新依赖）；文案里说清「会发生什么」。
import * as Dialog from "@radix-ui/react-dialog";
import type { ReactNode } from "react";
import { Button } from "./ui";

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
  confirmLabel = "确认",
  danger = false,
  children,
}: {
  open: boolean;
  title: string;
  onClose: () => void;
  onConfirm: () => void;
  confirmLabel?: string;
  danger?: boolean;
  /** 后果说明（渲染在正文区，允许 <b> 中性强调与 <Risk> 危险句）。 */
  children: React.ReactNode;
}) {
  return (
    <Dialog.Root open={open} onOpenChange={(o) => !o && onClose()}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 bg-black/60" />
        <Dialog.Content className="fixed left-1/2 top-1/2 w-[400px] max-w-[92vw] -translate-x-1/2 -translate-y-1/2 rounded-xl border border-slate-700 bg-slate-900 p-5 shadow-2xl">
          <Dialog.Title className="text-base font-semibold">{title}</Dialog.Title>
          <Dialog.Description className="mt-3 text-xs leading-relaxed text-ink-3 [&_b]:font-medium [&_b]:text-ink-2">
            {children}
          </Dialog.Description>
          <div className="mt-5 flex justify-end gap-2">
            <Button onClick={onClose}>取消</Button>
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
