// 宏库状态（从 MacroLibrary 拆出，行数预算 §6.1）：动作面板各区块共用一份
// 列表 / 编辑器 / 虚拟键盘态。用户 2026-11-24 定稿的水平布局：动作库合并成
// **一个横贯左键列+中列的大框**——头部（库切换 / 名称 / 遇错即停 / 新建）、
// 主体两列（动作列表窄列 + 步骤列表宽列）、底部步骤类型芯片 + 录入区 + 保存行。
// 右框让给语音坞（VoiceDock）。步骤录入的类型分段跟着编辑器，虚拟键盘按需浮层，
// 两边共用这一份 mods/main 态。
import { useEffect, useMemo, useRef, useState } from "react";
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
} from "./macros";

export function useMacroBoard() {
  const qc = useQueryClient();
  const toast = useToast();
  const macros = useMacros();
  const [draft, setDraft] = useState<Macro | null>(null);
  const [dirty, setDirty] = useState(false);
  const [busy, setBusy] = useState(false);
  const [confirmDel, setConfirmDel] = useState(false);
  // 步骤录入态：组合修饰键 / 主键 + 虚拟键盘浮层开关（芯片不再分模式，点一个即落一步）。
  const [mods, setMods] = useState(0);
  const [main, setMain] = useState<number | null>(null);
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
    setKbdStep(null);
  };
  const create = () => {
    setDraft(emptyMacro());
    setDirty(true);
    setKbdStep(null);
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

  /** 追加一步并立刻进入原位编辑（用户 2026-11-25 拍板：点「运行命令」芯片即落一步，
      光标直接落在步骤上等用户打字，底栏不再摆一个平行输入框）。 */
  const addStepEditing = (step: Step, cancellable = false) => {
    const base =
      draft ?? (list[0] ? { ...list[0], steps: [...list[0].steps] } : null);
    if (!base) return;
    setDraft({ ...base, steps: [...base.steps, step] });
    setDirty(true);
    setEditStep(base.steps.length);
    pendingNew.current = cancellable ? base.steps.length : null;
  };
  /** 删掉第 i 步（行内 × 与「命令清空即删步」共用一个口）。 */
  const removeStep = (i: number) => {
    const base = draft ?? (list[0] ? { ...list[0], steps: [...list[0].steps] } : null);
    if (!base || !base.steps[i]) return;
    setDraft({ ...base, steps: base.steps.filter((_, j) => j !== i) });
    setDirty(true);
    if (pendingNew.current === i) pendingNew.current = null;
  };
  /** 原位替换第 i 步（双击编辑的提交口，用户 2026-11-25）。 */
  const replaceStep = (i: number, step: Step) => {
    const base = draft ?? (list[0] ? { ...list[0], steps: [...list[0].steps] } : null);
    if (!base || !base.steps[i]) return;
    const steps = [...base.steps];
    steps[i] = step;
    setDraft({ ...base, steps });
    setDirty(true);
    pendingNew.current = null;
  };
  // 正在原位编辑的步骤下标（提到共享状态：底栏芯片加完一步要直接把它开成编辑态）。
  const [editStep, setEditStep] = useState<number | null>(null);
  // 「点芯片先落一步」留下的占位下标（系统功能 / 代理 CLI / 等待）：原位编辑没提交就收起
  // → 把这一步撤掉，库里不许留半成品。用 ref 不用 state：提交与收起在同一次事件里连着跑
  // （done = 提交 + 收起），state 还没落地就被读到旧值，会把刚提交那步删掉（实测过）。
  const pendingNew = useRef<number | null>(null);
  /** 原位编辑收起：刚追加的占位步骤没被提交就撤掉（提交口 replaceStep 会清 pendingNew）。 */
  const closeEdit = (i: number) => {
    setEditStep(null);
    if (pendingNew.current !== i) return;
    pendingNew.current = null;
    const base = draft ?? (list[0] ? { ...list[0], steps: [...list[0].steps] } : null);
    if (!base || !base.steps[i]) return;
    setDraft({ ...base, steps: base.steps.filter((_, j) => j !== i) });
    setDirty(true);
  };
  // 底栏浮层键盘改写中的步骤下标（用户 2026-11-25：快捷键步骤不再弹第二把键盘，
  // 统一走底栏那把）。null = 键盘在追加新步骤。
  const [kbdStep, setKbdStep] = useState<number | null>(null);
  /** 点主键即成一步：kbdStep 有值 → 原位改写那一步；否则追加（修饰键清掉接着拼下一条）。 */
  const commitKbd = (kvk: number) => {
    const step = actionStep({ type: "shortcut", shortcut: { mods, key: kvk } });
    if (kbdStep !== null) replaceStep(kbdStep, step);
    else addStep(step);
    setMods(0);
    setMain(null);
    setKbdStep(null);
  };
  /** 收键盘：连改写目标一起清掉，免得下次追加时误改上一步。 */
  const closeKbd = () => {
    setKbd(false);
    setKbdStep(null);
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

  /**
   * 整组挪位置（计划 2026-11-25 片③）：把排好的顺序交后端落 `order`。
   * 未保存草稿（id 为空）不进 ids——它还没身份，挪它没意义；剩下的相对顺序就是屏幕上的顺序。
   */
  const reorder = async (ordered: Macro[]) => {
    const ids = ordered.filter((mm) => mm.id).map((mm) => mm.id);
    if (!ids.length) return;
    try {
      await ipc("remote_macro_reorder", { ids });
      refresh();
    } catch (e) {
      toast.err(`${t("remote.macro.reorderFail")}：${errText(e)}`);
    }
  };

  /** 批量删（片③ 右键「删除」吃加选集）：逐条走既有删除口，顺带清掉引用它的键槽。 */
  const delMany = async (ids: string[]) => {
    if (!ids.length) return;
    setBusy(true);
    let cleared = 0;
    let fail = 0;
    for (const id of ids) {
      try {
        cleared += await ipc<number>("remote_macro_delete", { id });
      } catch {
        fail += 1;
      }
    }
    if (ids.includes(draft?.id ?? "")) {
      setDraft(null);
      setDirty(false);
    }
    if (fail) toast.err(`${t("remote.macro.deleteFail")}（${fail} / ${ids.length}）`);
    else toast.ok(t("remote.macro.deletedMany").replace("{n}", String(cleared)));
    setBusy(false);
    setConfirmDel(false);
    refresh();
    void qc.invalidateQueries({ queryKey: ["remote-buttons"] });
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

  // 防抖自动保存（用户 2026-11-25 拍板：取消保存钮）：脏了且够格（有名字有步骤）
  // 静置 800ms 即落库；不够格（新建还没起名等）不触发，保存钮撤了也不丢改动。
  const saveRef = useRef(save);
  saveRef.current = save;
  const eligible = !!draft && dirty && draft.name.trim() !== "" && draft.steps.length > 0;
  useEffect(() => {
    if (!eligible) return;
    const id = setTimeout(() => void saveRef.current(), 800);
    return () => clearTimeout(id);
  }, [eligible, draft]);

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
    reorder,
    delMany,
    save,
    del,
    mods,
    setMods,
    main,
    kbd,
    setKbd,
    addStep,
    addStepEditing,
    closeEdit,
    removeStep,
    commitKbd,
    closeKbd,
    replaceStep,
    editStep,
    setEditStep,
    kbdStep,
    setKbdStep,
  };
}

export type MacroBoardState = ReturnType<typeof useMacroBoard>;
