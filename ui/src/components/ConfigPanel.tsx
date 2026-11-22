// 右面板配置编辑器（UI-U-D）：固定右侧、默认 520px、左缘拖拽调宽、可折叠成竖条。
// 扁平标签（一个白名单文件一枚下划线标签）、默认只读（点「编辑」解锁）、格式化尽力而为。
// 写前三道闸沿用后端 takeover_write_file：格式校验 + 自动快照 + 指纹比对，界面不拼路径。
// 接管页旧内联编辑器已在新版验收后撤除（决策12 落地），这里就是唯一的配置编辑入口。
import { useCallback, useEffect, useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useToast } from "./Toast";
import { QueryError } from "./QueryError";
import { Button } from "./ui";
import { Tabs } from "./Tabs";
import { ClaudeSwitches } from "./ClaudeSwitches";
import { IconChevron, IconClose, IconWand } from "./icons";
import { ipc } from "../lib/ipc";
import { t, ts } from "../lib/i18n";

interface FileBrief {
  index: number;
  path: string;
  home: string;
  format: string;
  exists: boolean;
  size: number;
  mtime_ms: number;
  sha: string;
}

interface FileDoc {
  index: number;
  path: string;
  format: string;
  exists: boolean;
  content: string;
  sha: string;
  masked: boolean;
}

const W_KEY = "cfg-panel-w";
const MIN_W = 380;
const MAX_W = 900;
const DEFAULT_W = 520;

function loadWidth(): number {
  const n = Number(localStorage.getItem(W_KEY) ?? 0);
  return n >= MIN_W && n <= MAX_W ? n : DEFAULT_W;
}

interface Props {
  app: string;
  /** claude 语义开关需要的托管态（其它端忽略）。 */
  managed: boolean;
  onClose: () => void;
  /** 面板当前实际占用的横向宽度（fixed 浮层会盖住页面内容，主内容按它挤压让位）。 */
  onOccupy?: (w: number) => void;
}

export function ConfigPanel({ app, managed, onClose, onOccupy }: Props) {
  const qc = useQueryClient();
  const [width, setWidth] = useState(loadWidth);
  const [collapsed, setCollapsed] = useState(
    () => localStorage.getItem("cfg-panel-min") === "1",
  );
  const [picked, setPicked] = useState<number | null>(null);
  const [reveal, setReveal] = useState(false);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState("");
  const [base, setBase] = useState<FileDoc | null>(null);

  const toast = useToast();
  const dragRef = useRef(false);

  const files = useQuery<{ files: FileBrief[] }>({
    queryKey: ["takeover-files", app],
    queryFn: () => ipc<{ files: FileBrief[] }>("takeover_files", { app }),
  });
  const doc = useQuery<FileDoc>({
    queryKey: ["takeover-file", app, picked, reveal],
    queryFn: () => ipc<FileDoc>("takeover_read_file", { app, index: picked, reveal }),
    enabled: picked != null,
  });

  // 换端/换文件回到只读——「默认只读」是本面板的立身之本。
  useEffect(() => {
    setPicked(null);
    setEditing(false);
  }, [app]);

  // Esc 关闭（UI-AUDIT-3 P3-3）：本面板不是 Radix Dialog（停靠性质、可折叠常驻），
  // 自带同口径的键盘退出——原先能开不能关，键盘用户只能 Tab 摸「✕」。
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  // 让位上报：fixed 浮层盖住主内容必须让页面知道占了多宽（折叠竖带 36px = w-9）。
  // cleanup 报 0，面板卸载（含切视图）时主内容立即回位。
  useEffect(() => {
    if (!onOccupy) return;
    onOccupy(collapsed ? 36 : width);
    return () => onOccupy(0);
  }, [collapsed, width, onOccupy]);

  useEffect(() => {
    setEditing(false);
  }, [picked]);

  useEffect(() => {
    if (doc.data) {
      setDraft(doc.data.content);
      setBase(doc.data);
    }
  }, [doc.data]);

  // 拖左缘调宽：pointer 事件全程跟踪，松手落盘 localStorage。
  const startDrag = useCallback((e: React.PointerEvent) => {
    e.preventDefault();
    dragRef.current = true;
    const move = (ev: PointerEvent) => {
      if (!dragRef.current) return;
      // 上限三道：面板记忆上限、窗口 240px 保护带（UI-AUDIT-3 P3-3：窄窗不吞屏）、指针位置。
      setWidth(Math.max(MIN_W, Math.min(MAX_W, window.innerWidth - 240, window.innerWidth - ev.clientX)));
    };
    const up = (ev: PointerEvent) => {
      dragRef.current = false;
      localStorage.setItem(W_KEY, String(Math.max(MIN_W, Math.min(MAX_W, window.innerWidth - 240, window.innerWidth - ev.clientX))));
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }, []);

  const save = useMutation({
    mutationFn: () =>
      ipc<{ sha: string; bytes: number; backup_id: string | null }>("takeover_write_file", {
        app,
        index: picked,
        content: draft,
        // Tauri v2 命令参数一律按 camelCase 收（tauri-macros 强制转换），
        // 传 snake_case 会报 missing required key baseSha。
        baseSha: base?.sha ?? "",
      }),
    onSuccess: (r) => {
      toast.ok(
        ts("cfg.saved", {
          n: (r.bytes ?? 0).toLocaleString(),
          id: r.backup_id ?? t("cfg.noneBackup"),
        }),
      );
      setEditing(false);
      qc.invalidateQueries({ queryKey: ["takeover-file"] });
      qc.invalidateQueries({ queryKey: ["takeover-files"] });
      // 详情与矩阵都读配置真值（原 key "takeover-status" 不存在，属失效空转）。
      qc.invalidateQueries({ queryKey: ["takeover"] });
      qc.invalidateQueries({ queryKey: ["matrix"] });
      qc.invalidateQueries({ queryKey: ["takeover-backups"] });
    },
    onError: (e) => toast.err(ts("dlg.saveFail", { e: String(e) })),
  });

  const toggleCollapse = () => {
    setCollapsed((v) => {
      localStorage.setItem("cfg-panel-min", v ? "0" : "1");
      return !v;
    });
  };

  const dirty = base != null && draft !== base.content;
  const maskedBlock = base?.masked === true;
  const canFormat = (() => {
    if (!editing || !base || !base.format.toLowerCase().includes("json")) return false;
    try {
      JSON.parse(draft);
      return true;
    } catch {
      return false;
    }
  })();

  // 折叠态：一条竖带，点开设回。宽度记忆不受折叠影响。
  if (collapsed) {
    return (
      <button
        onClick={toggleCollapse}
        title={t("cfg.expandTip")}
        className="fixed right-0 top-0 z-40 flex h-full w-9 flex-col items-center gap-3 border-l border-slate-800 bg-slate-900/95 py-3 text-sm text-ink-3 hover:text-ink-1"
      >

        <IconChevron width={16} height={16} className="rotate-180" />
        <span className="tracking-widest [writing-mode:vertical-rl]">{ts("cfg.title", { app })}</span>
      </button>
    );
  }

  return (
    <aside
      className="fixed right-0 top-0 z-40 flex h-full max-w-[calc(100vw-240px)] flex-col border-l border-slate-800 bg-slate-900/95 backdrop-blur"
      style={{ width }}
    >
      {/* 左缘拖拽把手 */}
      <div
        onPointerDown={startDrag}
        title={t("cfg.dragTip")}
        className="absolute left-0 top-0 h-full w-1.5 cursor-col-resize hover:bg-cyan-500/40 active:bg-cyan-500/60"
      />

      <header className="flex items-center gap-2 border-b border-slate-800 px-4 py-2.5">
        <h3 className="text-base font-semibold text-ink-1">{ts("cfg.title", { app })}</h3>
        <div className="ml-auto flex items-center gap-1 text-sm text-ink-3">
          <button onClick={toggleCollapse} title={t("cfg.collapseTip")} className="rounded px-2 py-1 hover:bg-slate-800 hover:text-ink-1">
            {t("cfg.collapse")}
          </button>
          <button onClick={onClose} aria-label={t("cfg.closeTip")} className="grid h-7 w-7 place-items-center rounded text-ink-3 hover:bg-slate-800 hover:text-ink-1">
            <IconClose width={14} height={14} />
          </button>
        </div>
      </header>

      <div className="flex min-h-0 flex-1 flex-col overflow-y-auto px-4 py-3">
        {app === "claude" && (
          <div className="mb-3">
            <ClaudeSwitches managed={managed} />
          </div>
        )}

        <div className="flex flex-wrap items-end gap-x-4 gap-y-1 border-b border-slate-800 pb-px">
          <Tabs
            size="sm"
            ariaLabel={t("cfg.files")}
            value={String(picked ?? "")}
            onChange={(id) => setPicked(Number(id))}
            items={(files.data?.files ?? []).map((f) => ({
              id: String(f.index),
              title: `${f.path}（${f.format}）`,
              label: (
                <>
                  <span className="font-mono">{f.home.split("/").pop() || f.home}</span>
                  {!f.exists && <span className="ml-1 text-ink-4">{t("cfg.notExists")}</span>}
                </>
              ),
            }))}
          />
          {files.isLoading && <span className="text-sm text-ink-4">{t("cfg.readingList")}</span>}
          {/* 读取失败 ≠ 没有配置文件：原先失败时 tab 一行都没有，看着像白板 */}
          {files.isError && (
            <QueryError inline title={t("cfg.listReadFail")} error={files.error} onRetry={() => files.refetch()} />
          )}
        </div>

        {picked == null ? (
          <p className="pt-4 text-sm leading-relaxed text-ink-4">{t("cfg.pickFile")}</p>
        ) : (
          <div className="flex min-h-0 flex-1 flex-col pt-3">
            <div className="mb-2 flex flex-wrap items-center gap-x-3 gap-y-1 text-sm text-ink-3">
              <span className="break-all font-mono">{base?.path}</span>
              <span>{ts("cfg.fingerprint", { sha: base?.sha ?? "" })}</span>
              {base?.masked && (
                <span className="rounded bg-amber-500/15 px-1.5 py-0.5 text-[var(--sem-busy-text)]">{t("cfg.masked")}</span>
              )}
              <label className="flex items-center gap-1.5">
                <input type="checkbox" checked={reveal} onChange={(e) => setReveal(e.target.checked)} />
                {t("cfg.showKeys")}
              </label>
              <button className="underline decoration-slate-600 underline-offset-2" onClick={() => doc.refetch()}>
                {t("cfg.reread")}
              </button>
            </div>

            {/* 读取失败 ≠ 空文件：否则 draft 还停在上一个文件（或空串），
                用户会以为这个文件就是空的，甚至把旧内容当成新文件的。 */}
            {doc.isError ? (
              <div className="rounded-lg border border-slate-800 bg-slate-900 p-3">
                <QueryError inline title={t("cfg.readFail")} error={doc.error} onRetry={() => doc.refetch()} />
              </div>
            ) : (
            <textarea
              value={draft}
              readOnly={!editing}
              spellCheck={false}
              onChange={(e) => setDraft(e.target.value)}
              className={`min-h-[200px] w-full flex-1 resize-none rounded-md border p-3 font-mono text-xs leading-relaxed ${
                editing
                  ? "border-cyan-600/70 bg-slate-950 text-ink-2"
                  : "border-slate-800 bg-slate-950/60 text-ink-2"
              }`}
            />
            )}

            <div className="mt-2 flex flex-wrap items-center gap-3 text-sm">
              {editing ? (
                <>
                  <Button onClick={() => save.mutate()} disabled={!dirty || maskedBlock || save.isPending}>
                    {save.isPending ? t("cfg.writing") : t("dlg.save")}
                  </Button>
                  <Button
                    variant="ghost"
                    onClick={() => {
                      setDraft(base?.content ?? "");
                      setEditing(false);
                    }}
                  >
                    {t("cfg.exitEdit")}
                  </Button>

                  <Button
                    variant="ghost"
                    disabled={!canFormat}
                    title={canFormat ? t("cfg.formatTip") : t("cfg.formatBadTip")}
                    onClick={() => {
                      try {
                        setDraft(JSON.stringify(JSON.parse(draft), null, 2));
                      } catch (e) {
                        toast.err(ts("cfg.formatFail", { e: String(e) }));
                      }
                    }}
                  >
                    <IconWand width={13} height={13} className="inline-block align-[-2px]" />
                    {t("cfg.format")}
                  </Button>
                </>
              ) : (
                <Button onClick={() => setEditing(true)} disabled={!base}>
                  {t("row.edit")}
                </Button>
              )}
              <span className="text-ink-4">
                {maskedBlock
                  ? t("cfg.maskedNoSave")
                  : editing
                    ? dirty
                      ? t("cfg.unsaved")
                      : t("cfg.inSync")
                    : t("cfg.readOnly")}
              </span>
            </div>
          </div>
        )}
      </div>
    </aside>
  );
}
