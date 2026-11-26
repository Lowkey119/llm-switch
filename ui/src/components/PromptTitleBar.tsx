// 快速面板标题行（用户 2026-11-24/25）：遥控器连接状态 + 电量 + 语音实时波形 + 注入目标。
//
// 电量/连接状态与波形都复用遥控器页的共享件（remoteBits，用户 2026-11-25 拍板：
// 遥控器元素不另画第二套）；`remote_status` 一次查询（电量 null 显「—」，不造假数字）。
// 注入目标闪烁（用户 2026-11-24）：OK 注入时没有可注入目标，目标区红色闪烁提醒
// 先用鼠标切到目标 App 窗口再操作——错误必须长得像错误（法条：错误不得静默）。
import { useQuery } from "@tanstack/react-query";
import { ipc } from "../lib/ipc";
import { t } from "../lib/i18n";
import { RemoteConn, VBars, useVoiceLevels } from "../pages/remote/remoteBits";

interface RemoteStatus {
  battery: number | null;
  bound: { connected: boolean } | null;
}

const LEVELS = 60;

export default function PromptTitleBar({
  target,
  flash,
}: {
  /** 注入目标（面板弹前拿着焦点的 App）；null = 无目标。 */
  target: { name: string } | null;
  /** 无目标按 OK 触发的闪烁提醒（面板层置位，2s 自清）。 */
  flash: boolean;
}) {
  const st = useQuery({ queryKey: ["remote-status"], queryFn: () => ipc<RemoteStatus>("remote_status") });
  const levels = useVoiceLevels(LEVELS);
  return (
    <div className="flex items-center gap-3 px-3 pt-2 pb-1 text-lg">
      <span className="flex shrink-0 items-center gap-3 text-lg">
        <RemoteConn battery={st.data?.battery ?? null} live={!!st.data?.bound?.connected} />
      </span>
      {/* 语音波形：采音时实时跳动；没电平摆「—」，不画假波形。 */}
      {levels.length === 0 ? (
        <span className="flex h-6 min-w-0 flex-1 items-center justify-center rounded border border-slate-800 text-lg text-ink-4">
          —
        </span>
      ) : (
        <VBars levels={levels.slice(-30)} className="h-6 min-w-0 flex-1" />
      )}
      <span
        className={`shrink-0 rounded px-1.5 ${
          target ? "text-ink-3" : "text-[var(--sem-danger-text)]"
        } ${flash ? "animate-pulse bg-[var(--sem-danger)]/20 font-semibold" : ""}`}
        title={t("prompt.targetTip")}
      >
        {target ? `${t("prompt.targetTo")} ${target.name}` : t("prompt.targetPickHint")}
      </span>
    </div>
  );
}
