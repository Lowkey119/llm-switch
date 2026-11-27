// 遥控器共享读数件（用户 2026-11-25：快速面板复用遥控器页已有元素，不另画第二套）。
//
// 三件：电量+连接状态（遥控器页卡头状态条同款）、语音实时电平订阅、纵向波形柱
// （VoiceDock 同款口径：显示增益 4×，没电平摆「—」不画假波形）。
import { useEffect, useState } from "react";
import { IconBattery } from "../../components/icons";
import { barSelectCls } from "../../components/ui";
import { ipcEvent } from "../../lib/ipc";
import { t } from "../../lib/i18n";

/** 电量 + 连接状态（用户 2026-11-20 拍板口径：横置电池图标，≤20% 红、其余荧光绿；读不到显 —）。 */
export function RemoteConn({ battery, live }: { battery: number | null; live: boolean }) {
  return (
    <>
      <span
        className="tip flex items-center gap-1.5"
        data-tip={t("remote.battery")}
        aria-label={`${t("remote.battery")} ${battery != null ? `${battery}%` : "—"}`}
      >
        {battery != null ? (
          <b
            className={`flex items-center gap-1 ${
              battery <= 20 ? "text-[var(--sem-danger-text)]" : "text-[var(--sem-ok-text)]"
            }`}
          >
            <IconBattery level={battery / 100} className="h-5 w-9" />
            {battery}%
          </b>
        ) : (
          <b className="text-ink-1">—</b>
        )}
      </span>
      <span className="text-ink-3">
        {t("remote.connState")}{" "}
        <b className={live ? "text-[var(--sem-ok-text)]" : "text-ink-4"}>
          {live ? t("remote.connected") : t("remote.boundIdle")}
        </b>
      </span>
    </>
  );
}

/** 语音实时电平订阅（remote://voice，30ms 节流）：返回滚动窗尾部；状态事件（非电平）一到就清空。 */
export function useVoiceLevels(max: number): number[] {
  const [levels, setLevels] = useState<number[]>([]);
  useEffect(() => {
    let dead = false;
    let un: (() => void) | undefined;
    void ipcEvent<{ level?: number }>("remote://voice", (e) => {
      if (dead) return;
      if (typeof e.level === "number") {
        const lv = e.level;
        setLevels((prev) => [...prev.slice(-(max - 1)), lv]);
      } else {
        setLevels([]);
      }
    }).then((u) => {
      if (dead) u();
      else un = u;
    });
    return () => {
      dead = true;
      un?.();
    };
  }, [max]);
  return levels;
}

/**
 * 纵向波形柱：levels 尾部各画一根竖柱。显示增益 4×（设备麦原始电平低，说话约 0.1，
 * 不放大看着像直线）；没电平（空闲/未采）由调用方摆「—」，这里不画假波形。
 * 高度由 className 控制（VoiceDock h-16，标题行 h-6）。
 */
export function VBars({ levels, className }: { levels: number[]; className: string }) {
  return (
    <div
      role="img"
      aria-label={t("remote.voice.levelAria")}
      className={`flex items-end justify-end gap-px overflow-hidden rounded-md border border-slate-800 px-1 ${className}`}
    >
      {levels.map((v, i) => (
        <span
          key={i}
          className="w-1.5 shrink-0 rounded-[1px] bg-[var(--chart-1)]"
          style={{ height: `${Math.max(3, Math.min(1, v * 4) * 100)}%` }}
        />
      ))}
    </div>
  );
}

/** 底栏动作面板的两档（计划 2026-11-25）：`我的动作` = 宏库编辑，`快速面板` = quickpanel.json 的 tab 与内容。 */
export type BoardMode = "macros" | "quickpanel";

/**
 * 头部那颗组合框（用户 2026-11-25 拍板：原静态「我的动作」标签换成组合框）。
 * 仓内组合框口径 = 原生 `<select>`（同顶栏「预存方案」），不引第二套下拉组件。
 */
export function ModeSelect({ mode, onChange }: { mode: BoardMode; onChange: (m: BoardMode) => void }) {
  return (
    <select
      value={mode}
      onChange={(e) => onChange(e.target.value as BoardMode)}
      aria-label={t("remote.macro.libSwitch")}
      className={`h-full min-w-0 flex-1 cursor-pointer ${barSelectCls}`}
    >
      <option value="macros">{t("remote.macro.mineTitle")}</option>
      <option value="quickpanel">{t("remote.qp.title")}</option>
    </select>
  );
}
