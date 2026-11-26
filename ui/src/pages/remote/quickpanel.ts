// 快速面板 tab 配置的前端真相（`~/.llm-switch/quickpanel.json`）。
//
// 后端 `services/quickpanel.rs` 是唯一校验与写盘口（原子替换），这里只管编辑：
// 读 `quickpanel_config_get`，改完整体 `quickpanel_config_set` 落盘——**不做逐条 IPC**，
// 半份配置会让面板整个空掉，比不保存糟得多。落盘口径与动作面板一致：脏了静置 800ms 自动存。
//
// item 四种形状互斥（与后端 `QpItem::kind` 同一优先级 macro > action > cmd > t）：
// 文本 `{t,n}`｜宏引用 `{macro}`｜单动作 `{action}`｜命令 `{cmd}`。
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { errText } from "../../components/QueryError";
import { useToast } from "../../components/Toast";
import { ipc } from "../../lib/ipc";
import type { Action } from "./action";

export const QP_QUERY = ["quickpanel-config"];

export interface QpItem {
  /** 文本内容；命令条目没填时显示命令本身。 */
  t?: string;
  /** 备注（字符类的「打断」「清屏」）。 */
  n?: string;
  /** 引用「我的动作」里的一条宏（改动作同步生效，用户 2026-11-25 拍板：引用不是复制）。 */
  macro?: string;
  /** shell 命令。 */
  cmd?: string;
  /** 命令要不要面板两步确认。 */
  confirm?: boolean;
  /** 单个槽位动作（快捷键 / 打开应用 / 代理 CLI），形状与键位档里的动作一致。 */
  action?: Action;
}

export interface QpTab {
  key: string;
  label: string;
  type: string;
  dynamic?: string;
  sep?: boolean;
  items?: QpItem[];
}

export interface QpConfig {
  tabs: QpTab[];
  path: string;
  error: string | null;
}

/** `type` 白名单（与后端 `services/quickpanel.rs::TYPES` 同源，改一边必须改另一边）。 */
export const QP_TYPES = ["chars", "actions", "prompts", "skills", "config"] as const;

/**
 * 只读类型：条目由面板运行时给（扫盘、配置预览），或真相还在别处（`prompts` 读 `prompt_items`，
 * 随计划 ⑦ 片退役）——在编辑器里改它们没有落点，故一律只读并说明原因。
 */
export const QP_READONLY = ["prompts", "skills", "config"];

export const qpEditable = (tab: QpTab) => !QP_READONLY.includes(tab.type);

export type ItemKind = "macro" | "action" | "cmd" | "text";

/** 判执行种类（与后端同优先级）。全空 = null（后端会拒，这里也不许落盘）。 */
export function itemKind(it: QpItem): ItemKind | null {
  if (it.macro?.trim()) return "macro";
  if (it.action) return "action";
  if (it.cmd?.trim()) return "cmd";
  if (it.t?.trim()) return "text";
  return null;
}

/** 新 tab 的占位形状（key 由已有 key 推一个不撞的，后端只要求唯一非空）。 */
export function emptyTab(existing: QpTab[]): QpTab {
  let i = existing.length + 1;
  while (existing.some((x) => x.key === `tab${i}`)) i += 1;
  return { key: `tab${i}`, label: "", type: "chars", items: [] };
}

/** 防抖自动落盘的静置时长（与动作面板同一手感）。 */
const AUTOSAVE_MS = 800;

export function useQuickPanelBoard() {
  const qc = useQueryClient();
  const toast = useToast();
  const cfg = useQuery({ queryKey: QP_QUERY, queryFn: () => ipc<QpConfig>("quickpanel_config_get") });
  // 草稿：null = 还没到货（到货后一次性接管，之后的 refetch 不许覆盖用户正在改的东西）。
  const [tabs, setTabs] = useState<QpTab[] | null>(null);
  const [dirty, setDirty] = useState(false);
  const [busy, setBusy] = useState(false);
  const [sel, setSel] = useState(0);

  useEffect(() => {
    if (tabs === null && cfg.data) setTabs(cfg.data.tabs.map((x) => ({ ...x, items: [...(x.items ?? [])] })));
  }, [cfg.data, tabs]);

  const list = tabs ?? [];

  const patch = useCallback((next: QpTab[]) => {
    setTabs(next);
    setDirty(true);
  }, []);

  const save = useCallback(async () => {
    if (!tabs) return;
    setBusy(true);
    try {
      const saved = await ipc<QpTab[]>("quickpanel_config_set", { tabs });
      setTabs(saved);
      setDirty(false);
      void qc.invalidateQueries({ queryKey: QP_QUERY });
    } catch (e) {
      // 后端校验没过 = 一个字都没写进去（原子替换），原文件还是旧的，这里只说清哪不对。
      toast.err(errText(e));
    } finally {
      setBusy(false);
    }
  }, [tabs, qc, toast]);

  const saveRef = useRef(save);
  saveRef.current = save;
  // 够格才自动存（同动作面板的 `eligible` 口径）：后端拒空 label，新建 tab 还没起名时
  // 不该每静置 800ms 弹一次红条。没起名就挂着「未保存」，起了名自己落盘。
  const eligible = dirty && tabs !== null && tabs.every((x) => x.label.trim() !== "");
  useEffect(() => {
    if (!eligible) return;
    const id = setTimeout(() => void saveRef.current(), AUTOSAVE_MS);
    return () => clearTimeout(id);
  }, [eligible, tabs]);

  const shown = list[sel] ?? null;
  const items = useMemo(() => shown?.items ?? [], [shown]);

  const patchTab = (i: number, p: Partial<QpTab>) => patch(list.map((x, j) => (j === i ? { ...x, ...p } : x)));
  /** 整条换掉（不是字段合并）：改种类时旧字段必须干净掉，否则后端按优先级读出另一种。 */
  const setItem = (i: number, it: QpItem) =>
    patch(list.map((x, j) => (j === sel ? { ...x, items: (x.items ?? []).map((o, k) => (k === i ? it : o)) } : x)));
  /** 追加一条，回它的新下标（芯片加完要直接把光标交给那一行）。 */
  const addItem = (it: QpItem) => {
    if (!shown) return null;
    const at = shown.items?.length ?? 0;
    patch(list.map((x, j) => (j === sel ? { ...x, items: [...(x.items ?? []), it] } : x)));
    return at;
  };
  const removeItem = (i: number) =>
    patch(list.map((x, j) => (j === sel ? { ...x, items: (x.items ?? []).filter((_, k) => k !== i) } : x)));
  const moveItem = (i: number, d: number) =>
    patch(
      list.map((x, j) => {
        if (j !== sel) return x;
        const arr = [...(x.items ?? [])];
        const to = i + d;
        if (to < 0 || to >= arr.length) return x;
        [arr[i], arr[to]] = [arr[to], arr[i]];
        return { ...x, items: arr };
      }),
    );
  /** 整列换掉（批量挪位用：moveBy / moveEnd / dropAt 都是算好新数组喂回来）。 */
  const setItems = (next: QpItem[]) =>
    patch(list.map((x, j) => (j === sel ? { ...x, items: next } : x)));
  /** 批量删条目（与 `delTabs` 同口径：一次删干净，选中落到剩下的第一个）。 */
  const delItems = (idxs: number[]) => {
    const set = new Set(idxs);
    patch(list.map((x, j) => (j === sel ? { ...x, items: (x.items ?? []).filter((_, k) => !set.has(k)) } : x)));
    setSel(Math.max(0, Math.min(sel, list.length - 1)));
  };
  const addTab = () => {
    patch([...list, emptyTab(list)]);
    setSel(list.length);
  };
  /** 批量删 tab（多选删除）：一次删干净，选中落到剩下的第一个。 */
  const delTabs = (idxs: number[]) => {
    const set = new Set(idxs);
    patch(list.filter((_, j) => !set.has(j)));
    setSel(0);
  };
  const delTab = (i: number) => {
    patch(list.filter((_, j) => j !== i));
    setSel(Math.max(0, Math.min(i, list.length - 2)));
  };

  return {
    cfg,
    list,
    shown,
    items,
    sel,
    setSel,
    dirty,
    busy,
    save,
    patchTab,
    setItem,
    setItems,
    addItem,
    removeItem,
    moveItem,
    delItems,
    setList: patch,
    delTabs,
    addTab,
    delTab,
    editable: shown ? qpEditable(shown) : false,
  };
}

export type QuickPanelBoardState = ReturnType<typeof useQuickPanelBoard>;
