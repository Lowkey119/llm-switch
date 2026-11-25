// 接单开关的复用逻辑（用户 2026-11-18：顶栏不再摆独立按钮，改成**双击状态胶囊**切换）。
//
// 复用优先：与设置页开关卡、菜单栏开关是**同一个偏好、同一个查询键**（["status"] 的
// paused 字段 + set_paused 命令），这里不另起一份状态、不新造命令。App 本来就在轮询
// ["status"]，同键订阅不额外增加请求。
//
// 为什么从 components/AcceptToggle 抽到 lib：按钮撤了，逻辑得有个不挂在按钮上的家；
// 胶囊（components/StatusPill）与以后任何入口都复用同一个 hook，避免两处各写一份切换。
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ipc, type ShellStatus } from "./ipc";
import { t, ts } from "./i18n";
import { useToast } from "../components/Toast";

export function useAcceptToggle() {
  const qc = useQueryClient();
  const toast = useToast();
  const status = useQuery({
    queryKey: ["status"],
    queryFn: () => ipc<ShellStatus>("get_status"),
  });
  const pause = useMutation({
    mutationFn: (paused: boolean) => ipc<boolean>("set_paused", { paused }),
    onSuccess: (paused: boolean) => {
      // 乐观更新胶囊：status 轮询有间隔，不等它回来才变脸。
      qc.setQueryData(["status"], (old: ShellStatus | undefined) =>
        old ? { ...old, paused } : old,
      );
      qc.invalidateQueries({ queryKey: ["status"] });
      toast.ok(paused ? t("top.acceptPausedToast") : t("top.acceptResumedToast"));
    },
    onError: (e) => toast.err(ts("top.acceptToggleFail", { e: String(e) })),
  });

  const paused = !!status.data?.paused;
  const failed = status.isError;
  const tip = failed
    ? t("top.acceptReadFail")
    : pause.isPending
      ? t("top.acceptBusy")
      : paused
        ? t("top.acceptOff")
        : t("top.acceptOn");

  return {
    paused,
    failed,
    pending: pause.isPending,
    ready: !!status.data,
    tip,
    toggle: () => pause.mutate(!paused),
  };
}
