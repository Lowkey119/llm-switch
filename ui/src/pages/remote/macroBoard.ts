// 宏库状态（从 MacroLibrary 拆出，行数预算 §6.1）：动作面板各区块共用一份
// 列表 / 编辑器 / 虚拟键盘态。用户 2026-11-24 定稿的水平布局：动作库合并成
// **一个横贯左键列+中列的大框**——头部（库切换 / 名称 / 遇错即停 / 新建）、
// 主体两列（动作列表窄列 + 步骤列表宽列）、底部步骤类型芯片 + 录入区 + 保存行。
// 右框让给语音坞（VoiceDock）。步骤录入的类型分段跟着编辑器，虚拟键盘按需浮层，
// 两边共用这一份 mods/main 态。
import { useMemo, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { errText } from "../../components/QueryError";
import { useToast } from "../../components/Toast";
import { t } from "../../lib/i18n";
import { ipc } from "../../lib/ipc";
import type { Shortcut } from "./shortcut";
import {
  MACRO_QUERY,
  actionStep,
  emptyMacro,
  useMacros,
  type Macro,
  type Step,
  type StepKind,
} from "./macros";

export function useMacroBoard() {
  const qc = useQueryClient();
  const toast = useToast();
  const macros = useMacros();
  const [draft, setDraft] = useState<Macro | null>(null);
  const [dirty, setDirty] = useState(false);
  const [busy, setBusy] = useState(false);
  const [confirmDel, setConfirmDel] = useState(false);
  // 步骤录入态（类型 + 组合 + 等待毫秒 + 键盘折叠）
  const [stepKind, setStepKind] = useState<StepKind>("shortcut");
  const [mods, setMods] = useState(0);
  const [main, setMain] = useState<number | null>(null);
  const [delayMs, setDelayMs] = useState("500");
  const [kbd, setKbd] = useState(false);
  // 列表 = 已存动作 + 未保存草稿（id 为空）置顶：点「新建动作」必须立刻在列表里
  // 看到那条新动作（用户 2026-11-24 反馈），不然输入名字前根本不知道新动作在哪儿。
  const list = useMemo(() => {
    const saved = macros.data ?? [];
    return draft && draft.id === "" ? [draft, ...saved] : saved;
  }, [macros.data, draft]);
  const shown = draft ?? list[0] ?? null;

  const pick = (m: Macro) => {
    setDraft({ ...m, steps: [...m.steps] });
    setDirty(false);
  };
  const create = () => {
    setDraft(emptyMacro());
    setDirty(true);
  };
  const refresh = () => void qc.invalidateQueries({ queryKey: MACRO_QUERY });

  /** 追加一步。没选中任何宏时落到列表第一条（列表空则不动，步骤区会提示先新建）。 */
  const addStep = (step: Step) => {
    const base =
      draft ?? (list[0] ? { ...list[0], steps: [...list[0].steps] } : null);
    if (!base) return;
    setDraft({ ...base, steps: [...base.steps, step] });
    setDirty(true);
  };
  /** 点主键即成一步（与槽位直录同手感，修饰键清掉接着拼下一条）。 */
  const pressMain = (kvk: number) => {
    addStep(actionStep({ type: "shortcut", shortcut: { mods, key: kvk } }));
    setMods(0);
    setMain(null);
  };

  const save = async () => {
    if (!draft) return;
    setBusy(true);
    try {
      const saved = await ipc<Macro>("remote_macro_save", { def: draft });
      setDraft(saved);
      setDirty(false);
      toast.ok(t("remote.macro.saved"));
      refresh();
    } catch (e) {
      toast.err(`${t("remote.macro.saveFail")}：${errText(e)}`);
    } finally {
      setBusy(false);
    }
  };

  const del = async () => {
    if (!draft?.id) return;
    setBusy(true);
    try {
      const cleared = await ipc<number>("remote_macro_delete", { id: draft.id });
      toast.ok(t("remote.macro.deleted").replace("{n}", String(cleared)));
      setDraft(null);
      setDirty(false);
      refresh();
      // 引用被清掉的键位映射要回读，否则页面上还挂着已删的宏。
      void qc.invalidateQueries({ queryKey: ["remote-buttons"] });
    } catch (e) {
      toast.err(`${t("remote.macro.deleteFail")}：${errText(e)}`);
    } finally {
      setBusy(false);
      setConfirmDel(false);
    }
  };

  /** 系统功能预设 → 载入编辑器成一条未保存的动作（改名/加步骤后再保存）。 */
  const loadPreset = (name: string, sc: Shortcut) => {
    setDraft({ ...emptyMacro(), name, steps: [actionStep({ type: "shortcut", shortcut: sc })] });
    setDirty(true);
  };

  return {
    loadPreset,
    macros,
    list,
    shown,
    draft,
    setDraft,
    dirty,
    setDirty,
    busy,
    confirmDel,
    setConfirmDel,
    pick,
    create,
    save,
    del,
    stepKind,
    setStepKind,
    mods,
    setMods,
    main,
    delayMs,
    setDelayMs,
    kbd,
    setKbd,
    addStep,
    pressMain,
  };
}

export type MacroBoardState = ReturnType<typeof useMacroBoard>;
