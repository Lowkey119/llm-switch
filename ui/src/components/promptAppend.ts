// 「追加到 tab」——浮窗只读口径下唯一的写回入口（用户 2026-11-25 拍板）。
// 从 PromptPanel 拆出（该文件过了 ui-budget 的 550 行线，法条是「超预算先拆，不往里塞」）。
//
// 右栏那块多行文本**只是注入前的临时编辑**（语音补一句、字符芯片拼一下），不落库；
// 想存成条目就一键写进用户指定的 tab（`quickpanel_config_set`）。
// 内容的增删改本身全在遥控器页 → 快速面板档。
import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { ipc } from "../lib/ipc";
import { t } from "../lib/i18n";
import type { QpTab } from "../pages/remote/quickpanel";

/** 配置查询的键——与 PromptPanel 的 configQ 同一份，写盘后靠它刷新。 */
export const QP_CONFIG_KEY = ["quickpanel-config"];

export function useAppendToTab(o: {
  cfgTabs: QpTab[];
  /** 当前 tab 的 key（默认追加目标）。 */
  curTabKey: string;
  /** 右栏当前装入的正文；只有 inject 态可追加，其余传 null。 */
  stagedText: string | null;
  note: (msg: string, ms?: number) => void;
  /** 追加成功后的收尾（清右栏待执行）。 */
  onAppended: () => void;
}): { writableTabs: QpTab[]; target: string; setTarget: (k: string) => void; run: () => void } {
  const qc = useQueryClient();
  const [appendTab, setAppendTab] = useState("");
  // skills/config 这类只读档不收，写进去也没人读。
  const writableTabs = o.cfgTabs.filter((tb) => tb.type !== "skills" && tb.type !== "config");
  const target = writableTabs.some((x) => x.key === appendTab)
    ? appendTab
    : (o.curTabKey ?? writableTabs[0]?.key ?? "");
  const run = () => {
    const text = (o.stagedText ?? "").trim();
    if (!text || !target) {
      o.note(t("prompt.appendNoText"), 2500);
      return;
    }
    // 同一 tab 严格去重（后端写盘前也会兜一道）：这里先挡住，免得用户以为没加上。
    const dup = o.cfgTabs
      .find((tb) => tb.key === target)
      ?.items?.some((x) => (x.t ?? "").trim() === text);
    if (dup) {
      o.note(t("prompt.appendDup"), 2500);
      return;
    }
    const tabs = o.cfgTabs.map((tb) =>
      tb.key === target ? { ...tb, items: [...(tb.items ?? []), { t: text }] } : tb,
    );
    void ipc("quickpanel_config_set", { tabs })
      .then(() => {
        void qc.invalidateQueries({ queryKey: QP_CONFIG_KEY });
        o.onAppended();
        const who = o.cfgTabs.find((tb) => tb.key === target);
        o.note(`${t("prompt.appended")} ${who?.label ?? target}`, 3000);
      })
      .catch((e) => o.note(String(e), 4000));
  };
  return { writableTabs, target, setTarget: setAppendTab, run };
}
