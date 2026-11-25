// halogen/gufo 版本域（从 HalogenPanel 原样搬出）：ghcr 版本列表 query + 切换/备份/
// 恢复/删镜像四个 mutation + 下拉框状态与派生口径。
import { useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useToast } from "../../components/Toast";
import { every, POLL } from "../../lib/poll";
import { ipc, type HalOp, type HalVersions } from "../../lib/ipc";
import { t, ts } from "../../lib/i18n";
import { cmpVer, type Confirm } from "../halogen/fmt";

export function useHalVersions(ch: string) {
  const qc = useQueryClient();
  const toast = useToast();
  const [pickTag, setPickTag] = useState("");

  const versions = useQuery({
    queryKey: ["hal", ch, "versions"],
    queryFn: () => ipc<HalVersions>("halctl_versions", { ch }),
    refetchInterval: () => every(POLL.idle),
  });
  const switchVer = useMutation({
    mutationFn: (tag: string) => ipc<HalOp>("halctl_switch", { ch, tag }),
    onSuccess: () => {
      toast.ok(t("hl.switchOk"));
      qc.invalidateQueries({ queryKey: ["hal", ch] });
    },
    onError: (e) => toast.err(ts("hl.switchFail", { e: String(e) })),
  });
  const backup = useMutation({
    mutationFn: () => ipc<HalOp>("halctl_backup", { ch }),
    onSuccess: () => {
      toast.ok(t("hl.backupOk"));
      qc.invalidateQueries({ queryKey: ["hal", ch, "versions"] });
    },
    onError: (e) => toast.err(ts("set.backupFail", { e: String(e) })),
  });
  const restore = useMutation({
    mutationFn: () => ipc<HalOp>("halctl_restore", { ch }),
    onSuccess: () => {
      toast.ok(t("hl.restoreOk"));
      qc.invalidateQueries({ queryKey: ["hal", ch] });
    },
    onError: (e) => toast.err(ts("hl.restoreFail", { e: String(e) })),
  });
  // 删除本地镜像（备份引擎版本，腾磁盘）：删的是 tag，当前版本与运行中镜像由 102 端拒绝。
  const delImage = useMutation({
    mutationFn: (tag: string) => ipc<HalOp>("halctl_image_del", { ch, tag }),
    onSuccess: (r) => {
      if (r.ok) toast.ok(t("hl.imageDelOk"));
      else toast.err(t("hl.imageDelFailOut"));
      versions.refetch();
    },
    onError: (e) => toast.err(ts("hl.delFail", { e: String(e) })),
  });

  const v = versions.data;
  const latest = v && v.remote.length ? v.remote[v.remote.length - 1] : "";
  // 「有新版」必须比大小，不能只判不等：本地比线上新（如手动切过更新版）时
  // latest < current，旧写法会反过来提示「发现新版 0.18.23 → 0.18.22」的降级（实报）。
  const hasUpdate = Boolean(latest && v?.current && cmpVer(latest, v.current) > 0);
  const allTags = useMemo(() => {
    const set = new Set<string>([...(v?.remote ?? []), ...(v?.local ?? [])]);
    if (v?.current) set.add(v.current);
    return [...set].sort((a, b) => cmpVer(b, a));
  }, [v]);
  const localSet = new Set(v?.local ?? []);
  const chosenTag = pickTag || latest;

  /** 版本域确认动作的分发（confirm union 的版本分支；控制域分支在面板层）。 */
  const runVersionConfirm = (confirm: Confirm) => {
    if (confirm === "backup") backup.mutate();
    else if (confirm === "restore") restore.mutate();
    else if (confirm && typeof confirm === "object" && "delTag" in confirm)
      delImage.mutate(confirm.delTag);
    else if (confirm && typeof confirm === "object" && "startExclusive" in confirm) {
      /* startExclusive 属控制域，不该走到这里；兜底忽略。 */
    } else if (confirm && typeof confirm === "object") switchVer.mutate(confirm.switchTag);
  };

  return {
    versions,
    v,
    latest,
    hasUpdate,
    allTags,
    localSet,
    pickTag,
    setPickTag,
    chosenTag,
    switchVer,
    backup,
    restore,
    delImage,
    runVersionConfirm,
    /** 版本域四个动作的 pending（面板 busy 汇总用）。 */
    busyVer: switchVer.isPending || restore.isPending || backup.isPending || delImage.isPending,
    /** busyText 的版本分支（面板拼装用）。 */
    busyTextVer: switchVer.isPending
      ? t("hl.busySwitch")
      : delImage.isPending
        ? t("hl.busyDelImage")
        : restore.isPending
          ? t("hl.busyRestore")
          : backup.isPending
            ? t("set.backingUp")
            : "",
  };
}
