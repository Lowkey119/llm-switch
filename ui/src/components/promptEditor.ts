// 提示词面板右栏的编辑状态机（自 PromptPanel 拆出，行为一字不改）。
//
// 口径（用户 2026-11-19 三条拍板）：右栏常驻显示所选条目、**双击**才进编辑态、
// 改动防抖自动保存且失焦立即保存——没有手动「保存 / 取消」。
//   adding  = 底部「增」开的新建草稿（失焦时非空才落库，空稿直接作废）；
//   editing = 双击既有条目进入的编辑态。
// 拆出来是因为 PromptPanel 已过 ui-budget 的 550 行线，编辑这块是能独立读懂的
// 一个单元（状态 + 定时器 + 落盘时序），塞在页面里只会把它推得更远。
import { useEffect, useRef, useState, type RefObject } from "react";
import { ipc } from "../lib/ipc";

/** 正在编辑的目标：id 定住防列表重排串号，orig 做脏检查（没改就不发 IPC）。 */
interface EditTarget {
  id: number;
  orig: string;
}

export interface PromptEditorArgs {
  /** 当前选中条目；带 dir 的是技能（右侧只读文档），不进编辑。 */
  cur: { id: number; content: string; dir?: string } | undefined;
  /** 新建草稿落到哪个 tab。 */
  tab: string;
  /** 存完刷新列表（react-query 的 refetch，返回值不关心）。 */
  refetch: () => unknown;
  /** 右栏 textarea：「增」按钮点下后把焦点抢过来，否则第一个字会掉进「打字跳搜索」。 */
  taRef: RefObject<HTMLTextAreaElement>;
}

export interface PromptEditor {
  adding: boolean;
  editing: boolean;
  /** 编辑器可打字（新建或编辑中）：键盘口径据此把方向键让给文字光标。 */
  editorOpen: boolean;
  draft: string;
  /** 双击右栏进编辑态（技能条目、已在编辑/新建中不进）。 */
  beginEdit: () => void;
  startAdd: () => void;
  onDraftChange: (text: string) => void;
  onBlur: () => void;
  /** Esc 退出编辑：先落盘再退（与失焦同口径），新建空稿直接作废。 */
  cancelEdit: () => void;
  /** 立即落盘（失焦 / Esc 收窗时调用）：防抖窗口内没来得及存的改动在这里补齐。 */
  flush: () => Promise<void>;
}

export function usePromptEditor(a: PromptEditorArgs): PromptEditor {
  const [adding, setAdding] = useState(false);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState("");
  const draftRef = useRef("");
  const target = useRef<EditTarget | null>(null);
  const saveTimer = useRef<number | undefined>(undefined);

  const pushDraft = (text: string) => {
    draftRef.current = text;
    setDraft(text);
  };

  const flush = (): Promise<void> => {
    window.clearTimeout(saveTimer.current);
    const tg = target.current;
    const text = draftRef.current;
    if (!tg || !text.trim() || text === tg.orig) return Promise.resolve();
    tg.orig = text;
    return ipc("prompt_update", { id: tg.id, content: text })
      .then(() => {
        void a.refetch();
      })
      .catch(() => {});
  };

  /** 打字中防抖落库（800ms 静默即存）：只管存，不退编辑态——用户还在敲。 */
  const scheduleSave = () => {
    window.clearTimeout(saveTimer.current);
    saveTimer.current = window.setTimeout(() => void flush(), 800);
  };
  useEffect(() => () => window.clearTimeout(saveTimer.current), []);

  const cur = a.cur;
  const beginEdit = () => {
    if (adding || editing || !cur || cur.dir) return;
    pushDraft(cur.content);
    target.current = { id: cur.id, orig: cur.content };
    setEditing(true);
  };
  const startAdd = () => {
    pushDraft("");
    setAdding(true);
    a.taRef.current?.focus();
  };
  const onDraftChange = (text: string) => {
    pushDraft(text);
    // 新建不防抖（每跳一次就多一条），只在编辑既有条目时定时落库。
    if (!adding) scheduleSave();
  };
  const onBlur = () => {
    if (adding) {
      const text = draftRef.current;
      setAdding(false);
      pushDraft("");
      if (text.trim()) {
        void ipc("prompt_create", { tab: a.tab, content: text })
          .then(() => void a.refetch())
          .catch(() => {});
      }
      return;
    }
    if (editing) {
      // 存完再退编辑态：先退会把右栏闪回改动前的旧内容（缓存还没回灌）。
      // 带目标一致性校验——存的这几毫秒里若已经换到别的编辑对象，
      // 这次收尾就不许再动 editing / target，否则会把新对象踢出编辑态。
      const tg = target.current;
      void flush().then(() => {
        if (target.current === tg) {
          setEditing(false);
          target.current = null;
        }
      });
    }
  };

  const cancelEdit = () => {
    if (adding) {
      setAdding(false);
      pushDraft("");
      return;
    }
    if (editing) void onBlur();
  };

  // 选中项换人 = 编辑对象没了（点击换行会先走 onBlur 落盘，这里兜底列表重排）：
  // 草稿若还有未落盘的先补存，再退出编辑态，右栏立刻跟到新条目。
  useEffect(() => {
    if (target.current) void flush();
    setEditing(false);
    target.current = null;
    // 只认「选中项的 id 变了」这一次跳变。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [cur?.id]);

  return {
    adding,
    editing,
    editorOpen: adding || editing,
    draft,
    beginEdit,
    startAdd,
    onDraftChange,
    onBlur,
    cancelEdit,
    flush,
  };
}
