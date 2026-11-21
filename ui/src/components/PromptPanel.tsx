// 提示词速取浮动面板（计划 2026-11-18）。
//
// 全局快捷键（或托盘）唤起的置顶速取面板：tab = 通用 + 各代理，**回车**注入、
// 点击只选中（用户 2026-11-19 拍板：点击另有他用，注入只认回车）。
// 注入目标在面板弹出**之前**就拍成快照（谁拿着输入焦点就注给谁），顶行实时显示
// 「将注入 X」（剪贴板 → 收窗 → 焦点还给目标 App → 模拟系统粘贴）。窗口生死/
// 位置/尺寸/贴边圆钮全在 Rust 侧（gui::promptfloat），这里只画内容：
// 挂载时问一次形态（prompt_shape），之后吃 prompt://shape 广播换画圆钮。
//
// 键盘口径（计划 P0-4）：焦点不在输入框时 ←→ 换 tab、↑↓ 移动高亮、回车注入、
// Esc 收起；焦点在搜索框/编辑窗内时方向键回归文字光标。空手里直接打字会跳进
// 搜索框（速取面板的肌肉记忆）。
//
// 右栏口径（用户 2026-11-19 三条拍板）：右侧多行编辑器**常驻**——选中谁就实时
// 显示谁的内容，不管理论在不在编辑态；**双击**编辑框才进入编辑态；改动防抖自动
// 保存、焦点离开编辑框立即保存，没有手动「保存 / 取消」。底部只留 增 / 删。
//
// 手感与资源小窗同款：整窗拖拽起手 >4px 才交原生拖（macOS startDragging 吞
// pointerup，先起手就点不动了）；圆钮点击展开。
import { useEffect, useMemo, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { LogicalSize } from "@tauri-apps/api/dpi";
import { listen } from "@tauri-apps/api/event";
import { useQuery } from "@tanstack/react-query";
import { copyText } from "../lib/clipboard";
import { ipc, type ClientCol } from "../lib/ipc";
import { setLang, t, useI18n } from "../lib/i18n";
import { seedZoomFrom, useZoom } from "../lib/zoom";
import { usePromptKeys } from "./promptKeys";
import { usePromptEditor } from "./promptEditor";
import PromptRightPane from "./PromptRightPane";
import PromptTabRail from "./PromptTabRail";
import PromptFooter from "./PromptFooter";

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
  // 倍率变了顺带把窗口按同一比例缩放——那三处是量内容 refit，面板的尺寸归用户拖，
  // 所以按倍率换算；Rust 侧 Resized 落库照旧记住新尺寸。
  const zoomRef = useRef<number | null>(null);
  useZoom("prompt", (z) => {
    const prev = zoomRef.current;
    zoomRef.current = z;
    // 首帧只记账不动窗口：库存的尺寸本来就对应当前倍率，开局再乘一次会跳大小。
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

  // 注入目标与辅助功能授权都在 Rust 侧实时问，前端不猜：目标 = 面板弹前拿着输入
  // 焦点的那个 App（Rust 拍快照）；授权 = 当前这个构建能不能模拟按键。
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
  // Rust 盯到目标变了会广播（面板失焦时前端不轮询，不推就永远停在旧目标）。
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
    // refetch 稳定；targetQ/a11yQ 的 refetch 同理，不进依赖避免重挂。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [refetch]);

  const [tab, setTab] = useState(COMMON);
  const [query, setQuery] = useState("");
  const [hi, setHi] = useState(0);
  const searchRef = useRef<HTMLInputElement>(null);
  const taRef = useRef<HTMLTextAreaElement>(null);

  // 面板只摆四个 tab（用户 2026-11-19）：通用 / 命令 / 激活代理 / 项目接管。
  // 激活代理 = 注入目标那个终端里正跑着的那个代理；没认出代理就没这一档。
  // 其余代理的条目走顶部搜索（跳全部 tab）取，不再每家占一列把列表顶看不见。
  const agentTab = (clients.data ?? []).find((c) => c.app === target?.agent);
  const tabs = [
    { key: COMMON, label: t("prompt.tabCommon") },
    { key: SHELL, label: t("prompt.tabShell") },
    ...(agentTab ? [{ key: agentTab.app, label: agentTab.label }] : []),
    { key: SKILLS, label: t("prompt.tabSkills") },
  ];
  // 焦点切到哪个代理的终端就跟着开那个 tab（用户 2026-11-19）；目标没了或换成
  // 非代理 App 就退回通用。只看 agentKey 变没变，tabs 每帧新建不能进依赖。
  const agentKey = agentTab?.app ?? null;
  const autoTab = useRef<string | null>(null);
  useEffect(() => {
    if (agentKey === autoTab.current) return;
    autoTab.current = agentKey;
    setQuery("");
    setTab(agentKey ?? COMMON);
  }, [agentKey]);
  // 搜索态的结果要标归属，非激活代理的 tab 不在四个里，回 client_list 查名字。
  // 左侧竖排轨宽度有限，长名取短形式的规则在 PromptTabRail 里。
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
  // 技能清单：只在需要时取（搜索态也要能命中技能名）。
  const skillsQ = useQuery({
    queryKey: ["prompt-skills"],
    queryFn: () => ipc<SkillRow[]>("get_skills"),
    enabled: tab === SKILLS || !!q,
  });
  // 技能条目借同一套行渲染：id 取负数避开真实条目，`dir` 是右侧只读文档的键。
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
  const rows = useMemo<Row[]>(
    () => (skillRows.length ? [...promptRows, ...skillRows] : promptRows),
    [promptRows, skillRows],
  );
  const isSkills = tab === SKILLS;
  const cur = rows[hi];
  const curDir = cur?.dir;
  // 右侧只读文档：技能内容不许编辑（用户明确要求），所以是 <pre> 不是 textarea。
  // 搜索态也能选中技能，文档查询跟着选中行走，不再只限项目接管 tab。
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

  // 注入（计划 P0-3）：内容进剪贴板（用户手势上下文里 navigator.clipboard 才稳）
  // → Rust 收窗、焦点还给唤起前的 App、模拟系统粘贴。复制失败不收窗不注入。
  const [copiedId, setCopiedId] = useState<number | null>(null);
  const [note, setNote] = useState("");
  const act = (it: Row | undefined) => {
    if (!it) return;
    void copyText(it.content).then((ok) => {
      if (!ok) {
        setNote(t("prompt.copyFail"));
        window.setTimeout(() => setNote(""), 2000);
        return;
      }
      setCopiedId(it.id);
      if (it.dir) {
        // 技能名注入：没有 prompt_items 记录，取用计数与 refetch 都跳过。
        void ipc("prompt_inject").catch(() => void ipc("prompt_set_visible", { visible: false }));
        return;
      }
      // 记一次取用：这是「高频自动排前」的唯一数据源。计数失败不能影响注入，
      // 补一次 refetch 让顺序跟上（面板此刻正在收窗，看不到也无所。）
      void ipc("prompt_use", { id: it.id })
        .then(() => refetch())
        .catch(() => {});
      void ipc("prompt_inject").catch(() => void ipc("prompt_set_visible", { visible: false }));
    });
  };

  // 右栏编辑状态机（常驻显示 / 双击编辑 / 防抖 + 失焦自动保存）：拆在 promptEditor.ts。
  const ed = usePromptEditor({ cur, tab, refetch, taRef });
  const { adding, editing, draft, editorOpen, beginEdit, startAdd, onDraftChange, onBlur } = ed;
  // 右栏没有选中条目且不在新建时，摆「—」占位（法条：无数据一律 —，不造假内容）。
  const showDoc = !adding && !!curDir;
  // 右栏抬头：新建 / 技能文档 / 编辑中 / 常态（提示双击进入编辑）。
  let rightTitle = t("prompt.dblEdit");
  if (adding) rightTitle = t("prompt.addTitle");
  else if (showDoc) rightTitle = t("prompt.skillDoc");
  else if (editing) rightTitle = t("prompt.editing");

  // 删除两步确认（禁原生 confirm）：首击变成「确认删除」，2.5s 不回手自动撤销。
  const [armedDel, setArmedDel] = useState(false);
  const delTimer = useRef<number | undefined>(undefined);
  const del = () => {
    const it = rows[hi];
    if (!it) return;
    if (!armedDel) {
      setArmedDel(true);
      window.clearTimeout(delTimer.current);
      delTimer.current = window.setTimeout(() => setArmedDel(false), 2500);
      return;
    }
    setArmedDel(false);
    void ipc("prompt_delete", { id: it.id })
      .then(() => refetch())
      .catch(() => {});
  };
  useEffect(() => () => window.clearTimeout(delTimer.current), []);

  // 注入失败兜底：目标没拿到焦点或被系统挡住时，重开面板并提示手动粘贴。
  // 内容已经在剪贴板里，静默收窗等于把用户选的东西弄丢了。
  // 文案按原因分开说（用户实报「永远注入失败」：得一眼看出是没授权、没目标还是焦点没回来）。
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

  // 未授权时面板上直接给入口：点一下才让系统弹授权框（注入路径上绝不弹），
  // 弹完顺手复检一次状态（用户可能当场就勾了）。
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
    inject: () => act(rows[hi]),
    // Esc 收窗前先把右栏改动落盘：防抖窗口可能还没到点，直接收窗会丢字。
    flush: () => void ed.flush(),
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
    if (Math.hypot(e.clientX - a.x, e.clientY - a.y) < 4) return;
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
        className="float-orb select-none text-2xs font-medium"
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
        提
      </div>
    );
  }

  const hasTarget = rows.length > 0 && rows[hi] !== undefined;
  return (
    <div
      className="prompt-card flex h-full w-full flex-col select-none"
      onPointerDown={press}
      onPointerMove={drift}
      onPointerUp={release}
      onPointerCancel={release}
    >
      {/* 顶行：搜索（跨全部 tab）+ 收起。容器本身可拖窗，控件各自挡拖。 */}
      <div className="flex items-center gap-2 px-3 pt-2 pb-1">
        <input
          ref={searchRef}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder={t("prompt.search")}
          onPointerDown={(e) => e.stopPropagation()}
          className="min-w-0 flex-1 rounded-md bg-slate-800/60 px-2 py-1.5 text-sm text-ink-1 placeholder:text-ink-3"
        />
        {/* 将注入给谁（用户 2026-11-19）：唤起面板前拿着输入焦点的那个 App，一眼可见；
            旁边直接写「回车注入」——目标与动作得在同一处说得完，不能只靠底栏小字。 */}
        <span
          title={t("prompt.targetTip")}
          className={`flex shrink-0 items-center gap-1.5 text-xs ${
            target ? "text-ink-3" : "text-[var(--sem-danger-text)]"
          }`}
        >
          <span className="max-w-[9rem] truncate">
            {target ? `${t("prompt.targetTo")} ${target.name}` : t("prompt.targetNone")}
          </span>
          {target && (
            <kbd className="rounded border border-slate-600 bg-slate-800/80 px-1 py-px font-mono text-2xs text-ink-2">
              ⏎ {t("prompt.injectVerb")}
            </kbd>
          )}
        </span>
      </div>

      {/* 主体：左侧竖排 tab 轨 + 列表 + 右侧常驻多行编辑器（选中谁显示谁）。
          tab 放左侧竖排是用户明确要求：横排在代理多起来后换行三排，把列表顶到看不见。
          轨宽 96px（用户 2026-11-19 截图：68px 会把「OpenCode / CodeBuddy」截成
          「Open… / Code…」），文字左对齐，截不下的仍靠 title 读全名。 */}
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
        <div className="min-h-0 flex-1 overflow-y-auto px-2 pb-2">
          {items.isError ? (
            <div className="py-6 text-center text-xs text-ink-3">{t("prompt.loadFail")}</div>
          ) : rows.length === 0 ? (
            <div className="py-6 text-center text-xs text-ink-3">
              {items.isLoading ? t("prompt.loading") : q ? t("prompt.noHit") : t("prompt.empty")}
            </div>
          ) : (
            <ul className="space-y-1">
              {rows.map((it, i) => (
                <li key={it.id}>
                  <button
                    data-row={it.id}
                    className={`w-full rounded-lg px-2.5 py-2 text-left text-sm leading-relaxed text-ink-1 ${
                      i === hi
                        ? "bg-[var(--sel-bg)] ring-1 ring-inset ring-[var(--sel-ring)]"
                        : "hover:bg-slate-700/40"
                    }`}
                    title={copiedId === it.id ? t("prompt.copied") : t("prompt.clickSelect")}
                    onClick={() => setHi(i)}
                  >
                    <span className="line-clamp-2 whitespace-pre-wrap break-all">
                      {it.content}
                      {it.use_count > 0 && (
                        <span className="ml-1 text-2xs text-ink-4">×{it.use_count}</span>
                      )}
                    </span>
                    {q && (
                      <span className="mt-0.5 block text-xs text-ink-3">{tabLabel(it.tab)}</span>
                    )}
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>

        {/* 右栏：实现拆到 PromptRightPane（口径见那个文件头）。 */}
        <PromptRightPane
          title={rightTitle}
          doc={showDoc ? (docQ.data ?? t("prompt.skillDocLoading")) : null}
          empty={!adding && !cur}
          editable={adding || editing}
          value={adding || editing ? draft : (cur?.content ?? "")}
          taRef={taRef}
          onBeginEdit={beginEdit}
          onChange={onDraftChange}
          onBlur={onBlur}
        />
      </div>

      {/* 底栏：增 / 删 + 键盘提示 + 未授权时的授权入口（实现拆到 PromptFooter）。 */}
      <PromptFooter
        skillsTab={isSkills}
        hasTarget={hasTarget}
        armedDel={armedDel}
        note={note}
        a11yOk={a11yOk}
        onAdd={startAdd}
        onDel={del}
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
