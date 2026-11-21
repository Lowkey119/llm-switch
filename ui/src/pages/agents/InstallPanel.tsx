// 代理安装面板：扫描本机装了哪些代理、配置文件落点，没给一键装的给命令 + 一键复制，
// 能一键装的点一下就装（进度走后台任务队列，就地显示 + 任务托盘同步）。
//
// 设计口径：
// - 「已安装」= 可执行命中 或 配置存在；两者都展示，误判不至于把用户挡在外面。
// - 安装命令是后端固定白名单（绝不拼用户输入），这里既能一键执行也能复制自跑。
// - 进度：npm/curl 没有干净百分比，用不定进度条 + 实时末行文案（用户已认可）。
import { useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Button, StatusDot } from "../../components/ui";
import { IconCaret } from "../../components/icons";
import { useToast } from "../../components/Toast";
import { ipc, type AgentScan } from "../../lib/ipc";
import { every, POLL } from "../../lib/poll";
import { t, ts } from "../../lib/i18n";

interface Task {
  id: string;
  kind: string;
  label: string;
  status: "running" | "done" | "failed";
  progress: number;
  message: string;
}

/** 状态徽标：已装 vX / 装了但跑不起来 / 未检测到（只剩配置残留时不挂徽标）。 */
function Badge({ s }: { s: AgentScan }) {
  if (s.bin_found && s.broken)
    return (
      <span className="rounded border border-amber-500/40 bg-amber-500/10 px-1.5 py-0.5 text-2xs text-[var(--sem-busy-text)]">
        {t("ag.brokenBadge")}
      </span>
    );
  if (s.bin_found)
    return (
      <span className="rounded border border-emerald-500/40 bg-emerald-500/10 px-1.5 py-0.5 text-2xs text-[var(--sem-ok-text)]">
        {ts("ag.installedV", { v: s.version ?? s.bin })}
      </span>
    );
  // 「有配置 · 未找到命令」这块徽标按用户 2026-11-17 口径删掉：八家全挂着它，
  // 等于没说。配置在不在，下面「配置文件（N）」那一栏本来就看得见（● 存在 / ○ 缺失），
  // 不必再拿一个读不懂的复合词重复一遍；只剩配置、命令没命中时就不挂徽标。
  return (
    <span className="rounded border border-slate-700 bg-slate-800/60 px-1.5 py-0.5 text-2xs text-ink-4">
      {t("ag.notFound")}
    </span>
  );
}

/** 一行的安装卡（徽标 + 复制命令 + 一键装 + 就地进度）。代理详情页的门面节也复用它，
 * 免得「装 copilot」在两处各写一遍进度与文案。 */
export function InstallRow({ s, onInstalled }: { s: AgentScan; onInstalled: () => void }) {
  const toast = useToast();
  const qc = useQueryClient();
  const [taskId, setTaskId] = useState<string | null>(null);
  const [showPaths, setShowPaths] = useState(false);
  const [showCmd, setShowCmd] = useState(false);

  // 只在有在跑的安装任务时轮询任务表，装完即停（不常驻轮询）。
  const tasks = useQuery<Task[]>({
    queryKey: ["tasks"],
    queryFn: () => ipc<Task[]>("list_tasks"),
    refetchInterval: () => (taskId ? every(POLL.fast) : false),
  });
  const task = taskId ? tasks.data?.find((t) => t.id === taskId) : undefined;
  const running = task?.status === "running";

  const install = useMutation({
    mutationFn: () => ipc<string>("agent_install", { app: s.app }),
    onSuccess: (id) => {
      setTaskId(id);
    },
    onError: (e) => toast.err(ts("ag.installStartFail", { e: String(e) })),
  });

  // 任务收尾：成功刷新扫描（立刻见新版本）+ 失效接管态；失败弹错。
  useEffect(() => {
    if (task && task.status !== "running") {
      if (task.status === "done") {
        toast.ok(ts("ag.installDone", { label: s.label }));
        onInstalled();
        qc.invalidateQueries({ queryKey: ["takeover"] });
        qc.invalidateQueries({ queryKey: ["matrix"] });
      } else {
        toast.err(ts("ag.installFail", { label: s.label, msg: task.message }));
      }
      setTaskId(null);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [task?.status]);

  const copy = async () => {
    if (!s.install_command) return;
    try {
      await navigator.clipboard.writeText(s.install_command);
      toast.ok(t("ag.cmdCopied"));
    } catch {
      toast.err(t("ag.copyFail"));
    }
  };

  return (
    <div className="rounded-md border border-slate-800 bg-slate-900/40 px-3 py-2">
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
        <span className="min-w-0 truncate text-sm text-ink-1" title={s.label}>{s.label}</span>
        <Badge s={s} />
        {s.bin_path && (
          <span className="truncate font-mono text-2xs text-ink-4" title={s.bin_path}>
            {s.bin_path}
          </span>
        )}
        <span className="ml-auto flex shrink-0 items-center gap-1.5">
          {s.install_command && (
            <>
              <Button size="sm" variant="ghost" onClick={copy}>
                {t("ag.copyCmd")}
              </Button>
              <Button
                size="sm"
                variant="primary"
                disabled={running || install.isPending}
                onClick={() => install.mutate()}
              >
                {running ? t("ag.installing") : s.installed ? t("ag.reinstall") : t("pt.install")}
              </Button>
            </>
          )}
        </span>
      </div>

      {/* 安装进度：不定进度条 + 实时末行文案 */}
      {running && (
        <div className="mt-2">
          <div className="h-1 w-full overflow-hidden rounded bg-slate-700">
            <div
              className="h-full animate-pulse bg-cyan-400 transition-all"
              style={{ width: `${Math.max(8, Math.round((task?.progress ?? 0) * 100))}%` }}
            />
          </div>
          {task?.message && (
            <div className="mt-1 truncate text-sm text-ink-4" title={task.message}>
              {task.message}
            </div>
          )}
        </div>
      )}

      {/* 配置文件落点 + 安装命令（可展开、可复制自跑） */}
      <div className="mt-1.5 flex flex-wrap items-center gap-3 text-2xs text-ink-4">
        {s.config_paths.length > 0 && (
          <button type="button" className="inline-flex items-center gap-1 underline decoration-slate-700 underline-offset-2 hover:text-ink-2" onClick={() => setShowPaths((v) => !v)}>
            <IconCaret open={showPaths} />
            {ts("ag.configPathsN", { n: s.config_paths.length })}
          </button>
        )}
        {s.install_command && (
          <button type="button" className="inline-flex items-center gap-1 underline decoration-slate-700 underline-offset-2 hover:text-ink-2" onClick={() => setShowCmd((v) => !v)}>
            <IconCaret open={showCmd} />
            {t("ag.installCmd")}
          </button>
        )}
      </div>
      {showPaths && (
        <ul className="mt-1 space-y-0.5">
          {s.config_paths.map((c) => (
            <li key={c.path} className="flex items-center gap-1.5 font-mono text-2xs">
              {/* 存在/缺失是状态，不是装饰：走全站状态灯（形状 + sr-only），
                  原先那对 ●/○ 只有颜色与字形，读屏与灰度截图两头都读不出。 */}
              {c.exists ? <StatusDot state="healthy" text={t("ag.cfgExists")} /> : <StatusDot text={t("ag.cfgMissing")} />}
              <span className="truncate text-ink-3" title={c.path}>
                {c.path}
              </span>
            </li>
          ))}
        </ul>
      )}
      {showCmd && s.install_command && (
        <pre className="mt-1 overflow-x-auto whitespace-pre-wrap break-all rounded bg-slate-950/70 p-2 font-mono text-2xs text-ink-2">
          {s.install_command}
        </pre>
      )}
    </div>
  );
}

export function InstallPanel({ scans, onInstalled }: { scans: AgentScan[]; onInstalled: () => void }) {
  // 一行两列（用户 2026-11-17：「这页面最好一行安装两列代理来布局」）——
  // 八家一家一行要滚半屏，右边还空着大半。窄屏回退一列，卡内部布局不动。
  return (
    <div className="grid grid-cols-1 items-start gap-1.5 sm:grid-cols-2">
      {scans.map((s) => (
        <InstallRow key={s.app} s={s} onInstalled={onInstalled} />
      ))}
    </div>
  );
}
