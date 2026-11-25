// 动作添加步骤入口条（用户 2026-11-25 终稿）：贴在步骤清单底部，芯片就是「加一步」的
// 按钮——点一个即往清单落一步并就地开编辑（运行命令等打字 / 系统功能弹预设菜单 /
// 代理 CLI 组合框选代理后打参数 / 等待改毫秒 / 打开应用走系统选择框 / 快捷键弹底栏键盘）。
// appBusy/appErr 与虚拟键盘浮层归本组件。
import { useEffect, useState } from "react";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { errText } from "../../components/QueryError";
import { IconClock, IconHandPointUp, IconWand } from "../../components/icons";
import { t } from "../../lib/i18n";
import { ipc } from "../../lib/ipc";
import { ACTION_ICON, type AgentCli } from "./ActionForms";
import { FN_CATEGORIES } from "./FnPicker";
import { VirtualKeyboard } from "./VirtualKeyboard";
import type { AppEntry } from "./AppPicker";
import {
  STEP_KINDS,
  actionStep,
  delayStep,
  stepCount,
  type StepKind,
} from "./macros";
import type { MacroBoardState } from "./macroBoard";
import type { Macro } from "./macros";

export function MacroEditBar({
  b,
  onBind,
  boundCount = 0,
}: {
  b: MacroBoardState;
  onBind?: (m: Macro) => void;
  boundCount?: number;
}) {
  const m = b.shown;
  const [appBusy, setAppBusy] = useState(false);
  const [appErr, setAppErr] = useState<string | null>(null);
  const pickKind = (k: StepKind) => {
    if (k === "open_app") {
      setAppBusy(true);
      setAppErr(null);
      void ipc<AppEntry | null>("remote_app_pick")
        .then((e) => e && b.addStep(actionStep({ type: "open_app", bundle_id: e.bundle_id })))
        .catch((e) => setAppErr(errText(e)))
        .finally(() => setAppBusy(false));
      return;
    }
    if (k === "shortcut") {
      b.setKbdStep(null);
      b.setKbd(true);
      return;
    }
    // 运行命令（用户 2026-11-25 拍板）：点芯片 = 直接落一步并开原位编辑，光标落在步骤上
    // 等用户打字；底栏那个平行输入框删了（两处入口只会让人找不到光标在哪）。
    if (k === "run_command") {
      b.addStepEditing(actionStep({ type: "run_command", command: "" }));
      return;
    }
    // 系统功能（用户 2026-11-25 拍板：跟运行命令同一手感）：先落一步（拿首个预设当
    // 占位），原位立刻弹预设菜单挑；没挑就收起会被 closeEdit 撤掉，不留半成品。
    if (k === "sys") {
      b.addStepEditing(actionStep({ type: "shortcut", shortcut: FN_CATEGORIES[0].items[0].sc }), true);
      return;
    }
    // 等待（用户 2026-11-25 拍板：同一手感）：先落一步（500ms 占位），原位改毫秒；
    // 没改就收起会撤掉这一步。
    if (k === "delay") {
      b.addStepEditing(delayStep(500), true);
      return;
    }
    // 代理 CLI（同上）：先落一步（默认第一个可用代理），原位用组合框换代理，
    // 选完把光标交给参数框直接打参数。
    if (k === "agent_cli") {
      void ipc<AgentCli[]>("remote_agents_cli")
        .then((list) => {
          const a = list.find((x) => !x.broken) ?? list[0];
          if (!a) {
            setAppErr(t("remote.act.agentCliEmpty"));
            return;
          }
          b.addStepEditing(actionStep({ type: "agent_cli", app: a.app, args: "" }), true);
        })
        .catch((e) => setAppErr(errText(e)));
      return;
    }
  };
  // 虚拟键盘浮层的 Esc 收口：捕获阶段先于页面级 Esc。
  useEffect(() => {
    if (!b.kbd) return;
    const h = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      b.closeKbd();
    };
    window.addEventListener("keydown", h, true);
    return () => window.removeEventListener("keydown", h, true);
    // b 是整块宏面板态（每渲染重建），只按对本弹层有效的字段重挂监听；b 是参数不是
    // state，收进依赖等于每渲染重挂一次。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [b.kbd]);

  return (
    // 添加步骤入口条（用户 2026-11-25 拍板：芯片是「添加步骤的入口」，待在步骤清单底部）：
    // 作页脚贴在步骤列下方，弹层一律向上开。shrink-0（不是 flex-1）：flex-1 在 flex-col
    // 父容器里会沿纵轴把本条拉高，芯片上下各留一大片空白。
    <div className="relative flex w-full shrink-0 flex-wrap items-center gap-1.5 border-t border-slate-800 px-2 py-1.5">
      {/* 手形向上指（用户 2026-11-25 点名）：标明下面这些芯片是「添加步骤」的入口，
          点一个即往上方步骤清单落一步。tip 走 tip-up 向上弹（本条贴窗口底部，向下弹会被裁）。 */}
      <span
        className="tip tip-up shrink-0 pr-0.5 text-ink-4"
        data-tip={t("remote.macro.addStepHint")}
        role="img"
        aria-label={t("remote.macro.addStepHint")}
      >
        <IconHandPointUp className="h-4 w-4" />
      </span>
      {STEP_KINDS.map((k) => {
        const Icon = k.id === "delay" ? IconClock : k.id === "sys" ? IconWand : ACTION_ICON[k.id as Exclude<StepKind, "sys" | "delay">];
        return (
          <button
            key={k.id}
            type="button"
            disabled={k.id === "open_app" && appBusy}
            onClick={() => pickKind(k.id)}
            className="flex shrink-0 items-center gap-1.5 rounded-md px-2 py-1 text-sm text-ink-3 transition-colors hover:bg-slate-800 hover:text-ink-1 disabled:cursor-wait"
          >
            <Icon className="h-3.5 w-3.5 shrink-0" aria-hidden />
            {t(k.labelKey)}
          </button>
        );
      })}
          <span className="ml-auto flex shrink-0 items-center gap-1.5">
            {appErr && <span className="text-sm text-[var(--sem-danger-text)]">{appErr}</span>}
            {/* 步数计数（用户 2026-11-25：从头部行下移并加粗）：跟「添加步骤」入口同条，
                加了几步一眼就在眼前。 */}
            <span className="shrink-0 text-sm font-bold tabular-nums text-ink-1">
              {b.dirty ? t("remote.macro.unsaved") : stepCount(m?.steps.length ?? 0)}
            </span>
          </span>

      {/* 虚拟键盘浮层（用户 2026-11-25：向上弹）：盖住上方画布而不遮动作面板与右侧语音坞；
          拼完一个组合即收起，点外面或 Esc 也收。 */}
      {b.kbd && (
        <>
          <div className="fixed inset-0 z-40" onClick={b.closeKbd} />
          {/* 键盘浮层水平居中弹在窗口底部（用户 2026-11-25 拍板）：步骤列靠左，浮层
              若跟着列锚定，104 键一开就顶出窗口右沿（小键盘被裁）。104 键是 96 子列网格，
              窄于 ~1120px 时导航簇（PrtScn/ScrLk/Pause、Ins/Home/PgUp…）键帽装不下字形
              会逐字撞在一起，所以内层给最小宽、窗口不够宽就横滚，不缩键。 */}
          <div className="fixed bottom-3 left-1/2 z-50 max-h-[70vh] w-[1180px] max-w-[calc(100vw-1.5rem)] -translate-x-1/2 overflow-auto rounded-lg border border-slate-600 bg-slate-900 p-3 shadow-2xl">
            {b.kbdStep !== null && (
              <p className="mb-2 text-sm text-[var(--key-hl)]">
                {t("remote.macro.kbdRewrite").replace("{n}", String(b.kbdStep + 1))}
              </p>
            )}
            <div className="min-w-[1120px]">
              <VirtualKeyboard
                mods={b.mods}
                main={b.main}
                onMod={(bit) => b.setMods((v) => v ^ bit)}
                onMain={(kvk) => {
                  b.commitKbd(kvk);
                  b.closeKbd();
                }}
              />
            </div>
          </div>
        </>
      )}
      <ConfirmDialog
        open={b.confirmDel}
        title={`${t("remote.macro.delete")}「${b.draft?.name ?? ""}」`}
        onClose={() => b.setConfirmDel(false)}
        onConfirm={() => void b.del()}
        danger
        irreversible
        confirmLabel={t("remote.macro.delete")}
      >
        {t("remote.macro.deleteDesc")}
        {onBind && m && m.id !== "" && boundCount > 0 && (
          <p className="mt-2 text-sm text-[var(--sem-busy-text)]">
            {t("remote.macro.deleteBoundWarn").replace("{n}", String(boundCount))}
          </p>
        )}
      </ConfirmDialog>
    </div>
  );
}
