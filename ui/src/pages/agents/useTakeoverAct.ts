// 接管类动作（接管 / 回滚 / 还原备份 / 删除备份）的共用 mutation。
//
// 详情页和备份小节都要发这些命令，提示文案与刷新口径必须一致，
// 所以收成一处；两处各拿一份 useMutation 实例，互不干扰各自的 isPending。
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useToast } from "../../components/Toast";
import { ipc } from "../../lib/ipc";

export function useTakeoverAct() {
  const qc = useQueryClient();
  const toast = useToast();
  const refresh = () => {
    qc.invalidateQueries({ queryKey: ["takeover"] });
    qc.invalidateQueries({ queryKey: ["takeover-backups"] });
    // 列表上的接管状态点来自 matrix_get，一并刷新。
    qc.invalidateQueries({ queryKey: ["matrix"] });
  };

  return useMutation({
    mutationFn: ({ cmd, args }: { cmd: string; args?: Record<string, unknown> }) =>
      ipc(cmd, args),
    onSuccess: (res: unknown, vars) => {
      const r = res as { changed?: boolean };
      if (vars.cmd === "takeover_apply" && r?.changed === false) {
        toast.info("已经是接管态，没有需要改写的键");
      } else if (vars.cmd === "takeover_apply") {
        toast.ok("已接管：原件已备份，可随时逐字节回滚");
      } else if (vars.cmd === "takeover_revert") {
        toast.ok("已回滚到接管前的原文件");
      } else if (vars.cmd === "takeover_restore_backup") {
        toast.ok("已还原所选备份");
      } else if (vars.cmd === "takeover_delete_backup") {
        toast.ok("已删除备份");
      } else {
        toast.ok("已保存");
      }
      refresh();
    },
    onError: (e) => toast.err(`操作失败：${e}`),
  });
}
