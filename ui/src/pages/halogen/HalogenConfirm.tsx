// halogen 动作的二次确认闸门（计划 §6.1 自 HalogenPanel.tsx 拆出，行为不变）：
// 五种动作各自的后果文案，中断在飞流量的都按不可逆对待。
import { ConfirmDialog, Risk } from "../../components/ConfirmDialog";
import { t, tf, ts } from "../../lib/i18n";
import type { Confirm } from "./fmt";

export function HalogenConfirm({
  confirm,
  eng,
  paramFile,
  backupVer,
  backupWhen,
  onClose,
  onConfirm,
}: {
  confirm: Confirm;
  /** 引擎名（halogen / gufo）：确认标题跟着通道走，不再写死 halogen。 */
  eng: string;
  /** 参数文件名（halogen.env / gufo.conf）。 */
  paramFile: string;
  backupVer?: string;
  backupWhen?: string;
  onClose: () => void;
  onConfirm: () => void;
}) {
  return (
      <ConfirmDialog
        open={confirm != null}
        title={
          confirm === "stop"
            ? ts("hl.stopTitle", { eng })
            : confirm === "restart"
              ? ts("hl.restartTitle", { eng })
              : confirm === "backup"
                ? t("hl.backupTitle")
                : confirm === "restore"
                  ? t("hl.restoreConfirmTitle")
                  : confirm && typeof confirm === "object" && "delTag" in confirm
                    ? ts("hl.delImageTitle", { tag: confirm.delTag })
                    : confirm && typeof confirm === "object" && "startExclusive" in confirm
                      ? t("hl.startExclTitle")
                      : t("hl.switchTitle")
        }
        danger={
          confirm === "stop" ||
          confirm === "restart" ||
          confirm === "restore" ||
          (confirm != null && typeof confirm === "object" && "delTag" in confirm)
        }
        confirmLabel={t("common.confirm")}
        onClose={onClose}
        onConfirm={onConfirm}
      >
        {confirm === "stop" || confirm === "restart" ? (
          tf(confirm === "restart" ? "hl.restartBody" : "hl.stopBody", {
            risk: <Risk>{t("hl.riskInterrupt")}</Risk>,
          })
        ) : confirm === "backup" ? (
          tf("hl.backupBody", {
            file: paramFile,
            risk: <Risk>{t("hl.backupRisk")}</Risk>,
            note: backupVer
              ? ts("hl.backupNow", { ver: backupVer, when: backupWhen ?? "" })
              : t("hl.backupNone"),
          })
        ) : confirm === "restore" ? (
          tf("hl.restoreBody", {
            b: <b>{backupVer}</b>,
            risk: <Risk>{t("hl.restoreRisk")}</Risk>,
          })
        ) : confirm && typeof confirm === "object" && "delTag" in confirm ? (
          <>
            {tf("hl.delImageBody", {
              b: <b>{confirm.delTag}</b>,
              risk: <Risk>{t("hl.delImageRisk")}</Risk>,
            })}
            {backupVer === confirm.delTag ? (
              <>
                <br />
                <Risk>{ts("hl.delImageBackupNote", { tag: confirm.delTag })}</Risk>
                {t("hl.delImageBackupTail")}
              </>
            ) : null}
            <br />
            <span className="text-ink-4">{t("hl.delImagePodman")}</span>
          </>
        ) : confirm && typeof confirm === "object" && "startExclusive" in confirm ? (
          <>
            {tf("hl.exclBody", {
              b: <b>{t("hl.exclBold")}</b>,
              alt: confirm.startExclusive,
              risk: <Risk>{t("hl.exclRisk")}</Risk>,
            })}
            <br />
            <span className="text-ink-4">{t("hl.exclNote")}</span>
          </>
        ) : confirm && typeof confirm === "object" ? (
          tf("hl.switchBody", {
            b: <b>{confirm.switchTag}</b>,
            risk: <Risk>{t("hl.switchRisk")}</Risk>,
          })
        ) : null}
      </ConfirmDialog>
  );
}
