// 提示词速取浮动面板（计划 2026-11-18；2026-11-24/25 遥控器快速面板重设计）。
//
// 全局快捷键（或托盘）唤起的置顶速取面板。遥控器五键口径（用户 2026-11-24 拍板，
// 只用 ↑↓←→OK 完成增删改查注入，不碰鼠标与真实键盘）：
//   浏览态：↑↓ 选条目、←→ 换 tab、打字进右栏搜索框；
//     OK 短按 = 注入（无注入目标 → 标题行目标区红色闪烁提醒先切目标窗口）；
//     OK 长按 = 编辑选中条目（右栏切编辑态，语音直接说进去）。
//   编辑态：OK 短按 = 保存退出；OK 长按 = 待删确认，再长按 = 删除；Esc 退编辑。
//   列表尾钉「＋ 新增」行：OK 进入新建编辑——五键也能增条目。
// 右栏一框两用（用户 2026-11-24）：浏览态 = 搜索框（打字过滤），编辑态 = 内容编辑器。
// 标题行（用户 2026-11-25）：遥控器连接状态 + 电量 + 语音实时波形 + 注入目标，
// 全部复用遥控器页共享件（remoteBits），不另画第二套。
// 左侧按需弹虚拟符号键盘（语音打不准 `/`、`^C` 这类字符，点一下插进光标处）。
//
// 注入目标在面板弹出**之前**就拍成快照（谁拿着输入焦点就注给谁）；窗口生死/
// 位置/尺寸/贴边圆钮全在 Rust 侧（gui::promptfloat），这里只画内容。
// 手感与资源小窗同款：整窗拖拽起手 >4px 才交原生拖；圆钮点击展开。
import { useEffect, useMemo, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { LogicalSize } from "@tauri-apps/api/dpi";
import { listen } from "@tauri-apps/api/event";
import { useQuery } from "@tanstack/react-query";
import remoteImg from "../assets/remote-hi.webp";
import { copyText } from "../lib/clipboard";
import { ipc, type ClientCol } from "../lib/ipc";
import { setLang, t, useI18n } from "../lib/i18n";
import { seedZoomFrom, useZoom } from "../lib/zoom";
import { usePromptKeys } from "./promptKeys";
import { usePromptEditor } from "./promptEditor";
import PromptRightPane, { type RightMode } from "./PromptRightPane";
import PromptTabRail from "./PromptTabRail";
import PromptFooter from "./PromptFooter";
import PromptSymbolPad from "./PromptSymbolPad";
import PromptTitleBar from "./PromptTitleBar";

// 面板首次出现时跟主窗拿同一个倍率（用户：「面板字体太小，要同应用字体」）。
// 只在面板自己从没存过倍率时继承一次，之后它记自己的。
seedZoomFrom("prompt", "main");

interface PromptItem {
  id: number;
  tab: string;
  content: string;
  sort: number;
  /** 取用次数：高频条目自动排前（排序在 SQL 侧，前端只跟着画）。 */
  use_count: number;
  last_used_at: number;
}

const COMMON = "common";
/// 「通用命令」tab：终端里通用、不区分代理的那一份（git 等）。
/// 代理 tab 由 client_list 下发，COMMON 与 SHELL 是面板自己声明的两个。
const SHELL = "shell";
/** 「项目接管」tab：列技能名，选中即把名字注入；右侧只读显示技能文档。 */
const SKILLS = "skills";
/** 列表尾「＋ 新增」哨兵行 id（真实条目 id 从 1 起，0 不会撞）。 */
const ADD_ROW = 0;
type SkillRow = { dir: string; name: string; description?: string | null };
/** 列表行的实际形态：提示词条目，外加借同一套渲染的技能条目（带 `dir`）。 */
type Row = PromptItem & { dir?: string };

/** 模糊匹配：查询串逐字在内容里按序出现即命中（大小写不敏感）。 */
function fuzzy(content: string, q: string): boolean {
  const s = content.toLowerCase();
  let at = 0;
  for (const ch of q.toLowerCase()) {
    at = s.indexOf(ch, at);
    if (at < 0) return false;
    at += 1;
  }
  return true;
}

export default function PromptPanel() {
  useI18n();
  // 缩放手感与浮窗 / 速率线 / 资源小窗完全一致：Cmd+滚轮 / Cmd+±/ 复位 Cmd+0。
  const zoomRef = useRef<number | null>(null);
  useZoom("prompt", (z) => {
    const prev = zoomRef.current;
    zoomRef.current = z;
    if (prev === null || Math.abs(z - prev) < 0.001) return;
    void (async () => {
      try {
        const win = getCurrentWindow();
        const size = await win.innerSize();
        const scale = await win.scaleFactor();
        const ratio = z / prev;
        await win.setSize(
          new LogicalSize(
            Math.max(320, Math.round((size.width / scale) * ratio)),
            Math.max(200, Math.round((size.height / scale) * ratio)),
          ),
        );
      } catch {
        /* 拿不到尺寸就算了，字号照样跟着倍率走 */
      }
    })();
  });

  // 浮窗入口不走主窗 App，语言得自己取一次（缺省中文，与主窗同源偏好）。
  useEffect(() => {
    void ipc<string>("ui_lang_get")
      .then((l) => setLang(l === "en" ? "en" : "zh"))
      .catch(() => {});
  }, []);

  // 形态：面板 or 贴边圆钮（Rust 裁决，前端只跟画）。
  const [orb, setOrb] = useState(false);
  useEffect(() => {
    void ipc<{ orb: boolean }>("prompt_shape")
      .then((s) => setOrb(Boolean(s.orb)))
      .catch(() => {});
    const un = listen<{ orb: boolean }>("prompt://shape", (e) => setOrb(Boolean(e.payload.orb)));
    return () => void un.then((f) => f());
  }, []);

  const items = useQuery({ queryKey: ["prompt-items"], queryFn: () => ipc<PromptItem[]>("prompt_list") });
  const clients = useQuery({ queryKey: ["client-list"], queryFn: () => ipc<ClientCol[]>("client_list") });

  // 注入目标与辅助功能授权都在 Rust 侧实时问，前端不猜。
  const targetQ = useQuery({
    queryKey: ["prompt-target"],
    queryFn: () => ipc<{ name: string; agent: string | null } | null>("prompt_target"),
  });
  const a11yQ = useQuery({
    queryKey: ["prompt-a11y"],
    queryFn: () => ipc<boolean>("prompt_a11y_status"),
  });
  const target = targetQ.data ?? null;
  const a11yOk = a11yQ.data !== false;
  useEffect(() => {
    const un = listen("prompt://target", () => void targetQ.refetch());
    return () => void un.then((f) => f());
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 面板被唤起（获得焦点）就刷一次：条目可能在主窗改过，目标与授权状态也一样。
  const { refetch } = items;
  useEffect(() => {
    let un: (() => void) | undefined;
    let cancelled = false;
    void getCurrentWindow()
      .onFocusChanged(({ payload: focused }) => {
        if (!focused) return;
        void refetch();
        void targetQ.refetch();
        void a11yQ.refetch();
      })
      .then((f) => {
        if (cancelled) f();
        else un = f;
      });
    return () => {
      cancelled = true;
      un?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [refetch]);

  const [tab, setTab] = useState(COMMON);
  const [query, setQuery] = useState("");
  const [hi, setHi] = useState(0);
  const [padOpen, setPadOpen] = useState(false);
  // 无注入目标按 OK 的闪烁提醒（用户 2026-11-24）：2s 自清。
  const [flash, setFlash] = useState(false);
  const searchRef = useRef<HTMLTextAreaElement>(null);
  const taRef = useRef<HTMLTextAreaElement>(null);

  // 面板只摆四个 tab（用户 2026-11-19）：通用 / 命令 / 激活代理 / 项目接管。
  const agentTab = (clients.data ?? []).find((c) => c.app === target?.agent);
  const tabs = [
    { key: COMMON, label: t("prompt.tabCommon") },
    { key: SHELL, label: t("prompt.tabShell") },
    ...(agentTab ? [{ key: agentTab.app, label: agentTab.label }] : []),
    { key: SKILLS, label: t("prompt.tabSkills") },
  ];
  const agentKey = agentTab?.app ?? null;
  const autoTab = useRef<string | null>(null);
  useEffect(() => {
    if (agentKey === autoTab.current) return;
    autoTab.current = agentKey;
    setQuery("");
    setTab(agentKey ?? COMMON);
  }, [agentKey]);
  const tabLabel = (key: string) =>
    tabs.find((x) => x.key === key)?.label ??
    (clients.data ?? []).find((c) => c.app === key)?.label ??
    key;

  // 搜索态跨全部 tab 检索；普通态只看当前 tab。切换后高亮回到第一条。
  const q = query.trim();
  const promptRows = useMemo(() => {
    const all = items.data ?? [];
    if (!q) return all.filter((r) => r.tab === tab);
    return all.filter((r) => fuzzy(r.content, q));
  }, [items.data, tab, q]);
  const skillsQ = useQuery({
    queryKey: ["prompt-skills"],
    queryFn: () => ipc<SkillRow[]>("get_skills"),
    enabled: tab === SKILLS || !!q,
  });
  const skillRows = useMemo<Row[]>(() => {
    const list = skillsQ.data ?? [];
    const hit = (s: SkillRow) => !q || fuzzy(s.name, q) || fuzzy(s.description ?? "", q);
    const picked = q ? list.filter(hit) : tab === SKILLS ? list : [];
    return picked.map((s, i) => ({
      id: -(i + 1),
      content: s.name,
      tab: SKILLS,
      use_count: 0,
      sort: 0,
      last_used_at: 0,
      dir: s.dir,
    }));
  }, [skillsQ.data, tab, q]);
  // 列表尾钉「＋ 新增」哨兵行（技能 tab 只读，不摆）：五键也能增条目。
  const isSkills = tab === SKILLS && !q;
  const rows = useMemo<Row[]>(() => {
    const base = skillRows.length ? [...promptRows, ...skillRows] : promptRows;
    return isSkills ? base : [...base, { id: ADD_ROW, content: "", tab, use_count: 0, sort: 0, last_used_at: 0 }];
  }, [promptRows, skillRows, isSkills, tab]);
  const cur = rows[hi];
  const curDir = cur?.dir;
  // 选中行跟着键盘跑出视野就像「按了没反应」（用户 2026-11-25）：选中即滚进视口。
  useEffect(() => {
    if (!cur) return;
    document
      .querySelector(`[data-row="${CSS.escape(String(cur.id))}"]`)
      ?.scrollIntoView({ block: "nearest" });
  }, [cur]);
  const docQ = useQuery({
    queryKey: ["prompt-skill-doc", curDir ?? ""],
    queryFn: () => ipc<string>("read_skill_doc", { dir: curDir ?? "" }),
    enabled: !!curDir,
  });
  useEffect(() => setHi(0), [tab, q]);
  useEffect(() => {
    if (hi > rows.length - 1) setHi(Math.max(0, rows.length - 1));
  }, [rows.length, hi]);
  useEffect(() => {
    document.querySelector(`[data-row="${rows[hi]?.id}"]`)?.scrollIntoView({ block: "nearest" });
  }, [hi, rows]);

  // 注入（计划 P0-3）：内容进剪贴板 → Rust 收窗、焦点还给唤起前的 App、模拟系统粘贴。
  // 无注入目标（用户 2026-11-24）：不复制不收窗，目标区闪烁提醒先切目标窗口。
  const [copiedId, setCopiedId] = useState<number | null>(null);
  const [note, setNote] = useState("");
  const act = (it: Row | undefined) => {
    if (!it) return;
    if (it.id === ADD_ROW) {
      ed.startAdd();
      return;
    }
    if (!target) {
      setFlash(true);
      window.setTimeout(() => setFlash(false), 2000);
      return;
    }
    void copyText(it.content).then((ok) => {
      if (!ok) {
        setNote(t("prompt.copyFail"));
        window.setTimeout(() => setNote(""), 2000);
        return;
      }
      setCopiedId(it.id);
      if (it.dir) {
        void ipc("prompt_inject").catch(() => void ipc("prompt_set_visible", { visible: false }));
        return;
      }
      void ipc("prompt_use", { id: it.id })
        .then(() => refetch())
        .catch(() => {});
      void ipc("prompt_inject").catch(() => void ipc("prompt_set_visible", { visible: false }));
    });
  };

  // 右栏编辑状态机（防抖 + 失焦自动保存）：拆在 promptEditor.ts。
  const ed = usePromptEditor({ cur: cur && cur.id !== ADD_ROW ? cur : undefined, tab, refetch, taRef });
  const { adding, editing, draft, editorOpen, beginEdit, startAdd, onDraftChange, onBlur, cancelEdit } = ed;
  const showDoc = !adding && !!curDir;
  // 右栏模式（一框两用）：编辑/新建 → 编辑框；技能 → 只读文档；其余 → 搜索框。
  const rightMode: RightMode = editorOpen ? "edit" : showDoc ? "doc" : "search";
  let rightTitle = t("prompt.search");
  if (adding) rightTitle = t("prompt.addTitle");
  else if (showDoc) rightTitle = t("prompt.skillDoc");
  else if (editing) rightTitle = t("prompt.editing");

  // 删除两步确认（禁原生 confirm）：首击变成「确认删除」，2.5s 不回手自动撤销。
  // 编辑态长按 OK 也走这里（删的就是正在编辑的条目）。
  const [armedDel, setArmedDel] = useState(false);
  const delTimer = useRef<number | undefined>(undefined);
  const del = (it?: Row) => {
    const row = it ?? cur;
    if (!row || row.id === ADD_ROW || row.dir) return;
    if (!armedDel) {
      setArmedDel(true);
      window.clearTimeout(delTimer.current);
      delTimer.current = window.setTimeout(() => setArmedDel(false), 2500);
      return;
    }
    setArmedDel(false);
    void ipc("prompt_delete", { id: row.id })
      .then(() => refetch())
      .catch(() => {});
  };
  useEffect(() => () => window.clearTimeout(delTimer.current), []);

  // 符号盘插入（用户 2026-11-25）：编辑态插进编辑框光标处，否则追加进搜索框。
  const insertSym = (text: string) => {
    if (editorOpen) {
      const ta = taRef.current;
      if (ta) {
        const s = ta.selectionStart ?? draft.length;
        const e = ta.selectionEnd ?? s;
        const next = draft.slice(0, s) + text + draft.slice(e);
        onDraftChange(next);
        requestAnimationFrame(() => {
          ta.focus();
          ta.setSelectionRange(s + text.length, s + text.length);
        });
        return;
      }
    }
    setQuery((v) => v + text);
    searchRef.current?.focus();
  };

  // 注入失败兜底：目标没拿到焦点或被系统挡住时，重开面板并提示手动粘贴。
  useEffect(() => {
    const un = listen<{ reason?: string; target?: string | null }>("prompt://inject-failed", (e) => {
      const r = e.payload?.reason;
      const who = e.payload?.target;
      const msg =
        r === "no-permission"
          ? t("prompt.injectFailA11y")
          : r === "no-target"
            ? t("prompt.injectFailNoTarget")
            : r === "focus-lost" && who
              ? `${t("prompt.injectFailFocusTo")} ${who}`
              : t("prompt.injectFail");
      setNote(msg);
      window.setTimeout(() => setNote(""), 6000);
      void ipc("prompt_set_visible", { visible: true }).catch(() => {});
      void a11yQ.refetch();
    });
    return () => void un.then((f) => f());
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 未授权时面板上直接给入口：点一下才让系统弹授权框（注入路径上绝不弹）。
  const askA11y = () => {
    void ipc<boolean>("prompt_a11y_ask")
      .then((ok) => {
        setNote(ok ? t("prompt.a11yGranted") : t("prompt.a11yAskFail"));
        window.setTimeout(() => setNote(""), 5000);
      })
      .catch(() => {})
      .finally(() => void a11yQ.refetch());
  };

  usePromptKeys({
    rowCount: rows.length,
    hi,
    tab,
    tabs,
    q,
    editorOpen,
    searchRef,
    // OK 短按：编辑态 = 保存退出；浏览态 = 注入（无目标闪烁）。
    okShort: () => (editorOpen ? void onBlur() : act(rows[hi])),
    // OK 长按：编辑态 = 待删确认；浏览态 = 编辑选中条目（技能/新增行不进）。
    okLong: () => (editorOpen ? del(cur) : cur && cur.id !== ADD_ROW && !cur.dir ? beginEdit() : undefined),
    flush: () => void ed.flush(),
    cancelEdit: () => cancelEdit(),
    setHi,
    setTab,
  });

  // 整窗拖拽：起手 >4px 才交给原生拖（SysFloat 同款约束）。
  const armed = useRef<{ x: number; y: number; moved: boolean } | null>(null);
  const press = (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    armed.current = { x: e.clientX, y: e.clientY, moved: false };
  };
  const drift = (e: React.PointerEvent) => {
    const a = armed.current;
    if (!a || a.moved) return;
    if (Math.hypot(e.clientX - a.x, a.y - e.clientY) < 4) return;
    a.moved = true;
    void getCurrentWindow()
      .startDragging()
      .catch(() => {});
  };
  const release = () => {
    armed.current = null;
  };

  // 圆钮态：54px 圆钮（.float-orb 与浮窗/速率线/资源小窗同款）。没拖动就松手 = 点击展开。
  if (orb) {
    return (
      <div
        className="float-orb select-none"
        role="button"
        aria-label={t("prompt.orbTip")}
        title={t("prompt.orbTip")}
        onPointerDown={press}
        onPointerMove={drift}
        onPointerUp={() => {
          const a = armed.current;
          armed.current = null;
          if (a && !a.moved) void ipc("prompt_expand").catch(() => {});
        }}
        onPointerCancel={release}
      >
        <img src={remoteImg} alt="" aria-hidden="true" draggable={false} className="pointer-events-none h-10 w-auto" />
      </div>
    );
  }

  const hasTarget = rows.length > 0 && rows[hi] !== undefined && rows[hi].id !== ADD_ROW;
  return (
    <div
      className="prompt-card flex h-full w-full flex-col select-none"
      onPointerDown={press}
      onPointerMove={drift}
      onPointerUp={release}
      onPointerCancel={release}
    >
      {/* 标题行：遥控器状态 + 电量 + 语音波形 + 注入目标（复用遥控器页共享件）。 */}
      <PromptTitleBar target={target} flash={flash} />

      <div className="flex min-h-0 flex-1">
        <PromptTabRail
          tabs={tabs}
          tab={tab}
          searching={!!q}
          onPick={(k) => {
            setQuery("");
            setTab(k);
          }}
        />
        {/* 符号盘按需从左侧弹出（底栏「符号键盘」开关）。 */}
        {padOpen && <PromptSymbolPad onInsert={insertSym} />}
        <div className="min-h-0 flex-1 overflow-y-auto px-2 pb-2">
          {items.isError ? (
            <div className="py-6 text-center text-lg text-ink-3">{t("prompt.loadFail")}</div>
          ) : rows.length === 0 ? (
            <div className="py-6 text-center text-lg text-ink-3">
              {items.isLoading ? t("prompt.loading") : q ? t("prompt.noHit") : t("prompt.empty")}
            </div>
          ) : (
            <ul className="space-y-1">
              {rows.map((it, i) => (
                <li key={it.id}>
                  <button
                    data-row={it.id}
                    className={`w-full rounded-lg px-2.5 py-2 text-left text-xl leading-relaxed text-ink-1 ${
                      i === hi
                        ? "bg-[var(--sel-bg)] ring-1 ring-inset ring-[var(--sel-ring)]"
                        : "hover:bg-slate-700/40"
                    } ${it.id === ADD_ROW ? "text-ink-3" : ""}`}
                    title={copiedId === it.id ? t("prompt.copied" as never) : t("prompt.clickSelect")}
                    onClick={() => setHi(i)}
                  >
                    {it.id === ADD_ROW ? (
                      t("prompt.addNew")
                    ) : (
                      <>
                        <span className="line-clamp-2 whitespace-pre-wrap break-all">
                          {it.content}
                          {it.use_count > 0 && <span className="ml-1 text-base text-ink-4">×{it.use_count}</span>}
                        </span>
                        {q && <span className="mt-0.5 block text-base text-ink-3">{tabLabel(it.tab)}</span>}
                      </>
                    )}
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>

        {/* 右栏：一框两用（搜索 / 编辑），实现拆到 PromptRightPane。 */}
        <PromptRightPane
          mode={rightMode}
          title={rightTitle}
          doc={showDoc ? (docQ.data ?? t("prompt.skillDocLoading")) : ""}
          searchValue={query}
          onSearch={setQuery}
          searchRef={searchRef}
          value={adding || editing ? draft : (cur?.content ?? "")}
          taRef={taRef}
          editable={adding || editing}
          onBeginEdit={beginEdit}
          onChange={onDraftChange}
          onBlur={onBlur}
        />
      </div>

      <PromptFooter
        skillsTab={isSkills}
        hasTarget={hasTarget}
        armedDel={armedDel}
        note={note}
        a11yOk={a11yOk}
        padOpen={padOpen}
        onTogglePad={() => setPadOpen((v) => !v)}
        onAdd={startAdd}
        onDel={() => del()}
        onAskA11y={askA11y}
      />

      {/* 右下角缩放手把：原生 resize 拖（Rust 侧静默落库尺寸）。 */}
      <div
        className="absolute right-0 bottom-0 h-4 w-4 cursor-nwse-resize"
        title={t("prompt.resizeTip")}
        onPointerDown={(e) => {
          e.stopPropagation();
          void getCurrentWindow()
            .startResizeDragging("SouthEast")
            .catch(() => {});
        }}
      >
        <svg viewBox="0 0 16 16" className="h-full w-full text-ink-4" aria-hidden="true">
          <path d="M14 6 6 14M14 10l-4 4" stroke="currentColor" strokeWidth="1.2" fill="none" />
        </svg>
      </div>
    </div>
  );
}
