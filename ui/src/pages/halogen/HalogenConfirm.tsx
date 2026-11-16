// halogen 动作的二次确认闸门（计划 §6.1 自 HalogenPanel.tsx 拆出，行为不变）：
// 五种动作各自的后果文案，中断在飞流量的都按不可逆对待。
import { ConfirmDialog, Risk } from "../../components/ConfirmDialog";
import type { Confirm } from "./fmt";

export function HalogenConfirm({
  confirm,
  backupVer,
  backupWhen,
  onClose,
  onConfirm,
}: {
  confirm: Confirm;
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
            ? "停止 halogen？"
            : confirm === "restart"
              ? "重启 halogen？"
              : confirm === "backup"
                ? "存当前为备份？"
                : confirm === "restore"
                  ? "恢复备份？"
                  : confirm && typeof confirm === "object" && "delTag" in confirm
                    ? `删除本地镜像 ${confirm.delTag}？`
                    : confirm && typeof confirm === "object" && "startExclusive" in confirm
                      ? "启动本引擎？"
                      : "切换版本？"
        }
        danger={
          confirm === "stop" ||
          confirm === "restart" ||
          confirm === "restore" ||
          (confirm != null && typeof confirm === "object" && "delTag" in confirm)
        }
        confirmLabel="确认"
        onClose={onClose}
        onConfirm={onConfirm}
      >
        {confirm === "stop" || confirm === "restart" ? (
          <>
            这会<Risk>中断所有在飞请求</Risk>
            {confirm === "restart" ? "，重启后需等待引擎重新加载权重（约 1~5 分钟）。" : "。停止后可再手动启动。"}
          </>
        ) : confirm === "backup" ? (
          <>
            将把当前版本与 halogen.env 存入备份槽，<Risk>覆盖现有备份</Risk>
            {backupVer ? `（现为 ${backupVer}，${backupWhen}）` : "（当前还没有备份）"}。
          </>
        ) : confirm === "restore" ? (
          <>
            将恢复到备份版本 <b>{backupVer}</b>：<Risk>切换版本会重启引擎、中断所有在飞请求</Risk>（约 1~5 分钟）。
          </>
        ) : confirm && typeof confirm === "object" && "delTag" in confirm ? (
          <>
            将从 102 删除版本 <b>{confirm.delTag}</b> 对应的引擎镜像，
            <Risk>删完要重新拉取（约 3.6GB）才能切回来</Risk>。当前版本与正在运行的镜像，
            后端会直接拒绝删除。
            {backupVer === confirm.delTag ? (
              <>
                <br />
                <Risk>注意：{confirm.delTag} 正是备份槽指向的版本</Risk>
                ——删掉后「恢复备份」会走重新拉取。
              </>
            ) : null}
            <br />
            <span className="text-ink-4">
              若有已停止的容器引用该镜像，podman 会拒绝删除，输出里会写明是哪个容器。
            </span>
          </>
        ) : confirm && typeof confirm === "object" && "startExclusive" in confirm ? (
          <>
            两个引擎<b>同端口、内存只够一家</b>：正在运行的 {confirm.startExclusive} 会
            <Risk>先被停掉</Risk>，然后启动本引擎并等它就绪（加载权重约 3~5 分钟，期间推理不可用）。
            <br />
            <span className="text-ink-4">模型名不变，各代理配置一个字都不用改；起不来会自动回滚。</span>
          </>
        ) : confirm && typeof confirm === "object" ? (
          <>
            将切换到版本 <b>{confirm.switchTag}</b>
            ：缺镜像会先拉取（约 3.6GB，数分钟），随后<Risk>重启引擎并中断在飞请求</Risk>。
          </>
        ) : null}
      </ConfirmDialog>
  );
}
