// 仅 halogen：Flash-Next ↔ Qwen3.8-27B 切换开关。写 halogen.env 的 checkpoint/模型 ID 后重启生效；
// 键的白名单与写盘在 102 的 hal_env.py，这里只发值，不拼远端命令。
import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Toggle } from "../../components/ui";
import { ConfirmDialog, Risk } from "../../components/ConfirmDialog";
import { useToast } from "../../components/Toast";
import { ipc, type HalEnv, type HalOp } from "../../lib/ipc";
import { t, ts } from "../../lib/i18n";

const CK_27B = "/models/qwen38-27b-ht4.hgn";
const ID_27B = "halogen-qwen3.8-27b";
const ID_FLASH = "qwen3.8-flash-next";

export function ModelSwitch({ ch }: { ch: string }) {
  const qc = useQueryClient();
  const toast = useToast();
  const [ask, setAsk] = useState(false);
  // 与参数面板共用同一份 env 查询：任一侧保存后 invalidate 都会让这里回读到磁盘真值。
  const env = useQuery({
    queryKey: ["hal", ch, "env"],
    queryFn: () => ipc<HalEnv>("halctl_env", { ch }),
  });
  const is27 = (env.data?.values?.HALOGEN_CHECKPOINT ?? "") === CK_27B;

  const save = useMutation({
    mutationFn: async (to27: boolean) => {
      const w = await ipc<HalOp>("halctl_env_save", {
        ch,
        values: {
          HALOGEN_CHECKPOINT: to27 ? CK_27B : "",
          HALOGEN_MODEL_ID: to27 ? ID_27B : ID_FLASH,
        },
      });
      if (!w.ok) throw new Error(w.output || t("hl.modelSwitchFailRaw"));
      const r = await ipc<HalOp>("halctl_control", { ch, action: "restart" });
      if (!r.ok) throw new Error(r.output || t("hl.modelSwitchFailRaw"));
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["hal", ch] });
      toast.ok(t("hl.modelSwitchDone"));
    },
    onError: (e) => toast.err(ts("hl.modelSwitchFail", { e: String(e) })),
    onSettled: () => setAsk(false),
  });

  return (
    <>
      <div className="flex items-center gap-2">
        <span className={`text-xs ${is27 ? "text-cyan-300" : "text-ink-3"}`}>
          {is27 ? t("hl.switchToFlash") : t("hl.switchTo27b")}
        </span>
        <Toggle checked={is27} disabled={save.isPending || !env.data} onChange={() => setAsk(true)} />
        {save.isPending && (
          <span className="text-xs text-[var(--sem-busy-text)]">{t("hl.modelSwitchBusy")}</span>
        )}
      </div>
      <ConfirmDialog
        open={ask}
        title={t("hl.modelSwitchTitle")}
        danger
        confirmLabel={t("common.confirm")}
        onClose={() => setAsk(false)}
        onConfirm={() => save.mutate(!is27)}
      >
        {ts("hl.modelSwitchBody", {
          from: is27 ? t("hl.model27b") : t("hl.modelFlash"),
          to: is27 ? t("hl.modelFlash") : t("hl.model27b"),
        })}
        <br />
        <Risk>{t("hl.modelSwitchRisk")}</Risk>
      </ConfirmDialog>
    </>
  );
}
