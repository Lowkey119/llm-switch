// 顶栏遥控器快捷入口（用户 2026-11-19 两轮截图拍板）：双态图标显示连接/未连接，
// 同时是遥控器页的快速跳转入口——点一下直达，不用从边条绕。
//
// 双态不靠颜色单讲（UI 法条：状态不得只靠颜色）：图标着色之外，右上角小点
// 实心 = 已连接、空心圈 = 未连接，形状与颜色两路冗余；文字读数在 tooltip 与 aria。
// 只在 macOS 且已绑定遥控器时出现；连接态与遥控器页同源（remote_status 共享缓存，
// 5s 轮询走 POLL.status 档，与状态胶囊同节奏）。读不到/未绑定整颗不渲染，不占位。
import { useEffect, useRef } from "react";
import { useQuery } from "@tanstack/react-query";
import { isMac } from "../lib/key";
import { t } from "../lib/i18n";
import { ipc, ipcEvent } from "../lib/ipc";
import { every, POLL } from "../lib/poll";
import { useToast } from "./Toast";
import { IconRemote } from "./icons";

interface RemoteStatusLite {
  bound: { connected: boolean } | null;
}

export function RemoteChip({ onGo }: { onGo: () => void }) {
  const toast = useToast();
  const toastRef = useRef(toast);
  toastRef.current = toast;
  // 槽位动作失败（C8）：命令写错、App 没装这类错必须当场看见，不许按了没反应。
  // 挂在顶栏这颗钮上而不是遥控器页里——遥控器是全局外设，人在哪个页都得收得到。
  const q = useQuery({
    queryKey: ["remote-status"],
    queryFn: () => ipc<RemoteStatusLite>("remote_status"),
    enabled: isMac,
    refetchInterval: () => every(POLL.status),
  });
  useEffect(() => {
    let dead = false;
    let un: (() => void) | undefined;
    void ipcEvent<{ action: string; error: string }>("remote://action-error", (e) => {
      if (dead) return;
      toastRef.current.err(`${t("remote.act.failed").replace("{a}", e.action)}：${e.error}`);
    }).then((off) => (dead ? off() : (un = off)));
    return () => {
      dead = true;
      un?.();
    };
  }, []);
  const bound = q.data?.bound;
  if (!bound) return null;
  const live = bound.connected;
  const tip = `${t("remote.chipTip")} · ${live ? t("remote.connected") : t("remote.boundIdle")} · ${t("remote.chipGo")}`;
  return (
    <button
      type="button"
      onClick={onGo}
      data-tip={tip}
      aria-label={tip}
      className="tip grid h-8 w-8 place-items-center rounded-lg border border-slate-700 bg-slate-800/60 transition-colors hover:border-cyan-500/50"
    >
      <span className="relative">
        <IconRemote width={16} height={16} className={live ? "text-[var(--sem-ok-text)]" : "text-ink-4"} />
        <span
          aria-hidden
          className={`absolute -right-1.5 -top-1.5 h-2 w-2 rounded-full ${
            live ? "bg-[var(--sem-ok)]" : "border border-ink-4"
          }`}
        />
      </span>
    </button>
  );
}
