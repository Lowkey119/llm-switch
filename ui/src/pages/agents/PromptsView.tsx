// 代理页 · 提示词与命令（用户 2026-11-18）：浮动快速面板的内容与行为统一在这里管。
//
// 为什么开在代理页而不是设置页：条目本来就是「各家代理的常用命令」，和代理是同一件事；
// 设置页那张 PromptCard 已整体迁到这里，同一件事不留第二个入口（AGENTS 复用优先）。
//
// 三块：① 分类条目增删改（与浮窗同一张表，改完浮窗立刻生效）；② 唤起快捷键；
// ③ 注入方式（光标处插入 / 清空后替换）+ 辅助功能授权引导。
import { useMemo, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button, Card, CardHead, inputCls } from "../../components/ui";
import { PromptCard } from "../settings/PromptCard";
import { useToast } from "../../components/Toast";
import { ipc } from "../../lib/ipc";
import { t } from "../../lib/i18n";

interface PromptRow {
  id: number;
  tab: string;
  content: string;
  sort: number;
  use_count: number;
  last_used_at: number;
}

/** 分类显示名：常用两类走词典（中英要分开），代理名原样最清楚。 */
function tabLabel(tab: string): string {
  if (tab === "common") return t("prompt.tabCommon");
  if (tab === "shell") return t("prompt.tabShell");
  return tab;
}

/** 分类顺序：常用在前，代理按名字排，切来切去顺序不会跳。 */
function sortTabs(tabs: string[]): string[] {
  const head = ["common", "shell"].filter((x) => tabs.includes(x));
  const rest = tabs.filter((x) => !head.includes(x)).sort();
  return [...head, ...rest];
}

export function PromptsView() {
  const toast = useToast();
  const qc = useQueryClient();
  const items = useQuery({
    queryKey: ["prompt-items"],
    queryFn: () => ipc<PromptRow[]>("prompt_list"),
  });
  const mode = useQuery({ queryKey: ["prompt-mode"], queryFn: () => ipc<string>("prompt_mode_get") });

  const tabs = useMemo(
    () => sortTabs([...new Set((items.data ?? []).map((r) => r.tab))]),
    [items.data],
  );
  const [tab, setTab] = useState<string>("common");
  const active = tabs.includes(tab) ? tab : (tabs[0] ?? "common");
  const rows = (items.data ?? []).filter((r) => r.tab === active);

  // 新增 / 改写 / 删除都走按钮，不藏快捷键（用户口径：增删改要看得见）。
  const [draft, setDraft] = useState("");
  const [editing, setEditing] = useState<{ id: number; text: string } | null>(null);
  // 删除要两步确认：误删一条手写半天的提示词比多点一次烦得多。
  const [confirmId, setConfirmId] = useState<number | null>(null);

  const reload = () => {
    void qc.invalidateQueries({ queryKey: ["prompt-items"] });
  };

  const add = async () => {
    const text = draft.trim();
    if (!text) return;
    try {
      await ipc("prompt_create", { tab: active, content: text });
      setDraft("");
      reload();
      toast.ok(t("prompts.added"));
    } catch (e) {
      toast.err(String(e));
    }
  };

  const save = async () => {
    if (!editing) return;
    try {
      await ipc("prompt_update", { id: editing.id, content: editing.text.trim() });
      setEditing(null);
      reload();
      toast.ok(t("prompts.saved"));
    } catch (e) {
      toast.err(String(e));
    }
  };

  const del = async (id: number) => {
    try {
      await ipc("prompt_delete", { id });
      setConfirmId(null);
      reload();
      toast.ok(t("prompts.deleted"));
    } catch (e) {
      toast.err(String(e));
    }
  };

  const setMode = async (next: string) => {
    try {
      await ipc("prompt_mode_set", { mode: next });
      mode.refetch();
    } catch (e) {
      toast.err(String(e));
    }
  };

  return (
    // 左分类右条目（U9）：分类栏宽走 clamp 随容器连续变化，不再在某个视口宽度上整排堆叠。
    <div className="split-nav gap-4">
      {/* 左：分类。条目多的分类在按钮上直接给数量，不用点进去数。 */}
      <Card>
        <CardHead title={t("prompts.cats")} />
        <div className="flex flex-col gap-1">
          {tabs.map((k) => {
            const n = (items.data ?? []).filter((r) => r.tab === k).length;
            return (
              <button
                key={k}
                type="button"
                aria-pressed={k === active}
                onClick={() => {
                  setTab(k);
                  setEditing(null);
                  setConfirmId(null);
                }}
                className={`flex items-center justify-between rounded-md px-2.5 py-1.5 text-left text-sm transition-colors ${
                  k === active
                    ? "bg-slate-800 text-ink-1"
                    : "text-ink-2 hover:bg-slate-800/60 hover:text-ink-1"
                }`}
              >
                <span className="truncate">{tabLabel(k)}</span>
                <span className="ml-2 font-mono text-2xs text-ink-3">{n}</span>
              </button>
            );
          })}
        </div>
      </Card>

      <div className="flex min-w-0 flex-col gap-4">
        <Card>
          <CardHead title={`${t("prompts.items")} · ${tabLabel(active)}`} />
          <div className="flex flex-col gap-1.5">
            {rows.length === 0 && (
              <p className="py-3 text-sm text-ink-3">{t("prompts.emptyTab")}</p>
            )}
            {rows.map((r) => (
              <div
                key={r.id}
                className="flex items-start gap-2 rounded-md border border-slate-800 bg-slate-900/40 px-2.5 py-2"
              >
                {editing?.id === r.id ? (
                  <>
                    <textarea
                      className={`${inputCls} min-h-16 flex-1 font-mono text-2xs`}
                      value={editing.text}
                      autoFocus
                      onChange={(e) => setEditing({ id: r.id, text: e.target.value })}
                    />
                    <div className="flex shrink-0 flex-col gap-1">
                      <Button onClick={() => void save()}>{t("prompts.save")}</Button>
                      <Button onClick={() => setEditing(null)}>{t("prompts.cancel")}</Button>
                    </div>
                  </>
                ) : (
                  <>
                    <span className="flex-1 whitespace-pre-wrap break-words text-sm text-ink-1">
                      {r.content}
                    </span>
                    <span className="shrink-0 font-mono text-2xs text-ink-3">
                      {r.use_count > 0 ? `×${r.use_count}` : ""}
                    </span>
                    <div className="flex shrink-0 gap-1">
                      <Button
                        onClick={() => {
                          setConfirmId(null);
                          setEditing({ id: r.id, text: r.content });
                        }}
                      >
                        {t("prompts.edit")}
                      </Button>
                      {confirmId === r.id ? (
                        <>
                          <Button onClick={() => void del(r.id)}>{t("prompts.confirm")}</Button>
                          <Button onClick={() => setConfirmId(null)}>{t("prompts.cancel")}</Button>
                        </>
                      ) : (
                        <Button onClick={() => setConfirmId(r.id)}>{t("prompts.del")}</Button>
                      )}
                    </div>
                  </>
                )}
              </div>
            ))}
            {/* 新增常驻列表末尾：与改写同一套输入样式，省一个弹窗层 */}
            <div className="mt-1 flex items-start gap-2">
              <textarea
                className={`${inputCls} min-h-16 flex-1 font-mono text-2xs`}
                placeholder={t("prompts.newPlaceholder")}
                value={draft}
                onChange={(e) => setDraft(e.target.value)}
              />
              <Button onClick={() => void add()} disabled={!draft.trim()}>
                {t("prompts.add")}
              </Button>
            </div>
          </div>
        </Card>

        {/* 注入方式：用户明确要「清空当前焦点还是插入当前光标」可选，默认插入不动原内容。 */}
        <Card>
          <CardHead title={t("prompts.modeTitle")} />
          <div className="flex flex-wrap items-center gap-2 text-sm">
            {[
              ["insert", t("prompts.modeInsert")],
              ["replace", t("prompts.modeReplace")],
            ].map(([key, label]) => (
              <button
                key={key}
                type="button"
                aria-pressed={(mode.data ?? "insert") === key}
                onClick={() => void setMode(key)}
                className={`rounded-md border px-2.5 py-1.5 transition-colors ${
                  (mode.data ?? "insert") === key
                    ? "border-cyan-500/60 bg-slate-800 text-cyan-300"
                    : "border-slate-700 bg-slate-800/60 text-ink-2 hover:border-cyan-500/40 hover:text-cyan-300"
                }`}
              >
                {label}
              </button>
            ))}
            <span className="text-2xs text-ink-3">{t("prompts.modeTip")}</span>
          </div>
        </Card>

        {/* 快捷键与辅助功能授权：自设置页整体迁入，这里就是唯一入口。 */}
        <PromptCard />
      </div>
    </div>
  );
}
