// 底栏带（用户 2026-11-25 终稿）：左侧动作序列编辑面板（列表 + 步骤清单 + 底部
// 添加步骤入口条 MacroEditBar），右侧语音录入面板固定宽并列。
// 虚拟键盘不是页签——按需浮动层，由「快捷键」芯片或步骤双击唤起，向上弹（MacroEditBar 内）。
import { t } from "../../lib/i18n";
import { MacroBoardPane } from "./MacroLibrary";
import type { MacroBoardState } from "./macroBoard";
import { VoiceDock } from "./VoiceDock";
import type { Macro } from "./macros";

export function BottomDock({
  macroB,
  onBind,
  bindTarget,
  boundCount,
  voiceAvailable,
  voiceWidth,
}: {
  macroB: MacroBoardState;
  /** 把选中动作绑到目标槽（选中键的单击槽）。 */
  onBind?: (m: Macro) => void;
  bindTarget?: string | null;
  boundCount?: number;
  voiceAvailable: boolean;
  /** 语音坞列宽 = 实物图列宽（imgW），与画布中列严丝合缝对齐。 */
  voiceWidth: number;
}) {
  return (
    <div className="flex h-full min-h-0 gap-3" aria-label={t("remote.map.tabMacro")}>
      <div className="flex min-h-0 min-w-0 flex-1 flex-col gap-2">
        <MacroBoardPane b={macroB} bare onBind={onBind} bindTarget={bindTarget} boundCount={boundCount} />
      </div>
      {voiceAvailable && (
        <div className="flex min-w-0 shrink-0 flex-col" style={{ width: voiceWidth }}>
          <VoiceDock bare />
        </div>
      )}
    </div>
  );
}
