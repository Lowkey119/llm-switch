// 代理页左列：按「多槽 / 单槽 / 已停用」分组的代理清单。
// 只负责选谁，不碰接管语义 —— 数据仍来自 matrix_get（与模型矩阵同一份清单，D24）。
import { useState } from "react";
import { QueryError } from "../../components/QueryError";
import type { AgentScan, ClientCol, MatrixClient } from "../../lib/ipc";

interface Props {
  multi: MatrixClient[];
  single: MatrixClient[];
  /** 停用的端不进 matrix 列表，改从列源取（片三）。 */
  off: ClientCol[];
  /** 已停用但仍处接管态（配置文件未回滚）的 app，用来打「接管中」角标。 */
  residual: Set<string>;
  /** 扫描未检测到的代理（折叠组）：点选可去详情，安装去「安装引导」页。 */
  notInstalled: AgentScan[];
  sel: string | null;
  onSelect: (app: string) => void;
  loading: boolean;
  listError?: unknown;
  onRetryList?: () => void;
  colsError?: unknown;
  onRetryCols?: () => void;
}

const rowCls = (on: boolean, dim: boolean) =>
  `flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-xs transition-colors ${
    on
      ? dim
        ? "bg-slate-800 text-[var(--sem-busy-text)]"
        : "bg-slate-800 text-cyan-300"
      : dim
        ? "text-ink-4 hover:bg-slate-800/50 hover:text-ink-2"
        : "text-ink-3 hover:bg-slate-800/50 hover:text-ink-2"
  }`;

function Group({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <div className="mb-3">
      <div className="mb-1 flex items-baseline gap-2 px-1">
        <span className="text-xs font-medium text-ink-3">{title}</span>
      </div>
      <div className="space-y-0.5">{children}</div>
    </div>
  );
}

export function AgentList({
  multi,
  single,
  off,
  residual,
  notInstalled,
  sel,
  onSelect,
  loading,
  listError,
  onRetryList,
  colsError,
  onRetryCols,
}: Props) {
  // 未检测到组默认折叠（用户拍板「折叠」）：不硬隐藏，防误判把已装代理挡在外面。
  const [showMiss, setShowMiss] = useState(false);
  // 分组依据：槽位制（claude 六档 / codebuddy 三档 / pi 主+小双槽）进多槽组，清单制进单槽组。
  const group = (title: string, rows: MatrixClient[]) =>
    rows.length > 0 && (
      <Group title={title}>
        {rows.map((c) => (
          <button key={c.app} onClick={() => onSelect(c.app)} className={rowCls(c.app === sel, false)}>
            <span
              className={`h-1.5 w-1.5 shrink-0 rounded-full ${
                c.taken_over ? "bg-emerald-400" : "bg-[var(--dot-idle)]"
              }`}
            />
            <span className="truncate" title={c.label}>{c.label}</span>
            {c.facade ? (
              /* 门面没有接管态可说（没有配置文件可写），说它「待接管」是误导——
                 它要说的是可见清单里现在有几个模型。 */
              <>
                <span className="shrink-0 rounded border border-slate-700 px-1 text-2xs text-ink-4">
                  门面
                </span>
                <span className="ml-auto shrink-0 text-2xs text-ink-4">
                  可见 {c.visible.length}
                </span>
              </>
            ) : (
              <>
                {c.slots.length > 0 && (
                  <span className="shrink-0 rounded border border-slate-700 px-1 text-2xs text-ink-4">
                    {c.tiers.length > 0 ? `${c.tiers.length + 1} 档` : `${c.slots.length} 槽`}
                  </span>
                )}
                <span
                  className={`ml-auto shrink-0 text-2xs ${
                    c.taken_over ? "text-[var(--sem-ok-text)]" : "text-ink-4"
                  }`}
                >
                  {c.taken_over ? "已接管" : "待接管"}
                </span>
              </>
            )}
          </button>
        ))}
      </Group>
    );

  return (
    <aside className="h-full w-full overflow-auto">
      {loading && <div className="px-1 text-xs text-ink-4">载入中…</div>}
      {listError != null && (
        <div className="px-1">
          <QueryError inline title="列表读取失败" error={listError} onRetry={onRetryList} />
        </div>
      )}
      {colsError != null && (
        <div className="px-1">
          <QueryError inline title="启停状态读取失败" error={colsError} onRetry={onRetryCols} />
        </div>
      )}
      {group("多槽代理", multi)}
      {group("单槽代理", single)}
      {notInstalled.length > 0 && (
        <div className="mb-3">
          <button
            type="button"
            onClick={() => setShowMiss((v) => !v)}
            className="mb-1 flex w-full items-baseline gap-2 px-1 text-left"
          >
            <span className="text-xs font-medium text-ink-3">
              {showMiss ? "▾" : "▸"} 未检测到（{notInstalled.length}）
            </span>
            <span className="text-xs text-ink-4">本机没扫到 · 去「安装引导」装</span>
          </button>
          {showMiss && (
            <div className="space-y-0.5">
              {notInstalled.map((s) => (
                <button key={s.app} onClick={() => onSelect(s.app)} className={rowCls(s.app === sel, true)}>
                  <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-slate-700" />
                  <span className="truncate" title={s.label}>{s.label}</span>
                  <span className="ml-auto shrink-0 text-2xs text-ink-4">未装</span>
                </button>
              ))}
            </div>
          )}
        </div>
      )}
      {off.length > 0 && (
        <Group title="已停用">
          {off.map((c) => (
            <button key={c.app} onClick={() => onSelect(c.app)} className={rowCls(c.app === sel, true)}>
              <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-slate-700" />
              <span className="truncate" title={c.label}>{c.label}</span>
              {residual.has(c.app) && (
                <span
                  className="shrink-0 rounded border border-amber-500/40 bg-amber-500/10 px-1 text-2xs text-[var(--sem-busy-text)]"
                  title="已停用但仍处接管状态（配置文件未回滚）"
                >
                  接管中
                </span>
              )}
              <span className="ml-auto shrink-0 text-2xs text-ink-4">已停用</span>
            </button>
          ))}
        </Group>
      )}
    </aside>
  );
}
