// 观测页（融合原型 unified.html 回灌）：实时监控 / 用量 / 日志三个原独立页并成一页三签。
// 三个页面组件零改动直接挂载；负上边距抵掉各页自带的顶部留白，避免页签行下双份空白。
import { useState } from "react";
import { Tabs } from "../components/Tabs";
import { MigrationNote } from "../components/MigrationNote";
import { t, tf } from "../lib/i18n";
import { MonitorPage } from "./Monitor";
import { UsagePage, USAGE_RANGES, type UsageRangeKey } from "./Usage";
import { LogsPage } from "./Logs";

type ObserveTab = "live" | "usage" | "logs";

export function ObservePage() {
  const [tab, setTab] = useState<ObserveTab>("live");
  // 用量区间选择器上提到页签行右侧（用户 2026-11-21）：状态跟着放这里，Usage 只消费。
  const [range, setRange] = useState<UsageRangeKey>("today");
  return (
    <div className="space-y-3 pt-[var(--content-pad-top)]">
      {/* 一次性迁移提示（计划 4.3 同款惯例）：导航 9 并 5 对老肌肉记忆冲击最大，
          关掉一次即永久收起。 */}
      <MigrationNote id="ia-2026-11">
        {tf("nav.ia", {
          observe: <b>{t("nav.ia.observe")}</b>,
          console: <b>{t("nav.ia.console")}</b>,
          skills: <b>{t("nav.ia.skills")}</b>,
        })}
      </MigrationNote>
      <div className="flex items-center justify-between gap-3 border-b border-slate-800">
        <Tabs
          variant="segmented"
          size="sm"
          ariaLabel={t("observe.tabAria")}
          value={tab}
          onChange={(id) => setTab(id as ObserveTab)}
          items={[
            { id: "live", label: t("observe.tab.live") },
            { id: "usage", label: t("observe.tab.usage") },
            { id: "logs", label: t("observe.tab.logs") },
          ]}
        />
        {tab === "usage" && (
          <Tabs
            variant="segmented"
            size="sm"
            ariaLabel={t("usage.rangeAria")}
            value={range}
            onChange={(id) => setRange(id as UsageRangeKey)}
            items={USAGE_RANGES.map((r) => ({ id: r.key, label: t(r.labelKey) }))}
          />
        )}
      </div>
      {/* 各页自带顶部留白（--content-pad）叠上页签行后的间距会偏大；只小幅上提（-mt-2）。
          不用负 content-pad：那会把内容盒拉到页签行下方重叠，tab 下半截被透明层接走点击。 */}
      <div className="-mt-3">
        {tab === "live" && <MonitorPage />}
        {tab === "usage" && <UsagePage range={range} />}
        {tab === "logs" && <LogsPage />}
      </div>
    </div>
  );
}
