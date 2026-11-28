// 底栏带（用户 2026-11-25 终稿）：左侧动作序列编辑面板（列表 + 步骤清单 + 底部
// 添加步骤入口条 MacroEditBar），右侧语音录入面板固定宽并列。
// 虚拟键盘不是页签——按需浮动层，由「快捷键」芯片或步骤双击唤起，向上弹（MacroEditBar 内）。
import { useState } from "react";
import { t } from "../../lib/i18n";
import { MacroBoardPane } from "./MacroLibrary";
import type { MacroBoardState } from "./macroBoard";
import { QuickPanelBoard } from "./QuickPanelBoard";
import type { BoardMode } from "./remoteBits";
import { VoiceDock } from "./VoiceDock";

export function BottomDock({
  macroB,
  boundCount,
  voiceAvailable,
  voiceWidth,
  profileId,
}: {
  macroB: MacroBoardState;
  /** 当前映射里绑定了选中动作的键槽数（删除弹层警示用）。 */
  boundCount?: number;
  voiceAvailable: boolean;
  /** 当前遥控器档案：语音增益按设备单独调。 */
  profileId?: string | null;
  /** 语音坞列宽 = 实物图列宽（imgW），与画布中列严丝合缝对齐。 */
  voiceWidth: number;
}) {
  // 两档编辑（计划 2026-11-25）：`我的动作` = 宏库，`快速面板` = quickpanel.json 的 tab 与内容。
  // 两档共用同一套骨架（头部行 + 左列表 + 右清单 + 底部芯片），只是数据源与芯片不同。
  const [mode, setMode] = useState<BoardMode>("macros");
  return (
    <div className="flex h-full min-h-0 gap-3" aria-label={t("remote.map.tabMacro")}>
      <div className="flex min-h-0 min-w-0 flex-1 flex-col gap-2">
        {mode === "macros" ? (
          <MacroBoardPane b={macroB} bare boundCount={boundCount} mode={mode} onMode={setMode} />
        ) : (
          <QuickPanelBoard bare mode={mode} onMode={setMode} profileId={profileId} />
        )}
      </div>
      {voiceAvailable && (
        <div className="flex min-w-0 shrink-0 flex-col" style={{ width: voiceWidth }}>
          <VoiceDock bare profileId={profileId} />
        </div>
      )}
    </div>
  );
}
