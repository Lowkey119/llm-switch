// halogen/gufo 原生控制台：替代各自引擎的 iframe 入口。数据全部经后端 SSH 控制面
// （services::halctl）从 102 拉取——状态/指标/日志/启停/版本/参数一屏收纳。
// ch 选择机位（halogen→/mnt/haloen 一套脚本，gufo→/mnt/data/gufo 一套脚本）；
// 中文日志由 102 上的 hal_log_cn.py / gu_log_cn.py 渲染，这里只展示，渲染逻辑只有一份。
// 子件（halogen/ 目录）：useHalVersions 版本域 / LogsCard 日志卡 / HalogenParams 调参。
import { useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Button, Card, fieldCls } from "../components/ui";
import { useToast } from "../components/Toast";
import { HalogenParams } from "./HalogenParams";
import { every, POLL } from "../lib/poll";
import { ipc, type HalOp, type HalStatus } from "../lib/ipc";
import { fmt, fmtBytes, num, str, type Confirm } from "./halogen/fmt";
import { Stat } from "./halogen/parts";
import { HalogenConfirm } from "./halogen/HalogenConfirm";
import { useHalVersions } from "./halogen/useHalVersions";
import { LogsCard } from "./halogen/LogsCard";

export function HalogenPanel({
  ch = "halogen",
  // 配对后端信息（Console 传入）：halogen/gufo 同端口互斥，启动本引擎前若对面
  // 还在跑，必须先弹确认「停对面再起本引擎」（用户口径：切页面不切引擎，
  // 真正启动时才谈互斥）。未配对时不传，启动就是普通启动。
  backend,
  altEngine,
  onEngineSwitched,
}: {
  ch?: string;
  backend?: string;
  altEngine?: string;
  onEngineSwitched?: () => void;
}) {
  const qc = useQueryClient();
  const toast = useToast();
  const [confirm, setConfirm] = useState<Confirm>(null);
  // 日志卡双用途：默认日志，点「参数设置」临时切成调参窗口（用户拍板的复用方式）。
  const [view, setView] = useState<"logs" | "params">("logs");

  const status = useQuery({
    queryKey: ["hal", ch, "status"],
    queryFn: () => ipc<HalStatus>("halctl_status", { ch }),
    refetchInterval: () => every(POLL.fast),
  });

  // 这几个都是「按了没反应就像没点上」的引擎级动作，失败必须说出来（闸门矩阵 1.6）。
  const control = useMutation({
    mutationFn: (action: string) => ipc<HalOp>("halctl_control", { ch, action }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["hal", ch] });
    },
    onError: (e) => toast.err(`引擎操作失败：${e}`),
  });
  // 互斥启动：后端侧编排「先停在跑的 → 起本引擎 → 等就绪，起不来自动回滚」
  // （commands::halctl::halctl_use_engine），前端只管确认与收尾。
  const exclusiveStart = useMutation({
    mutationFn: () =>
      ipc<{ ok: boolean; engine: string }>("halctl_use_engine", {
        backend: backend ?? "",
        engine: ch,
      }),
    onSuccess: (r) => {
      toast.ok(`已停对面引擎，本引擎（${r.engine}）启动就绪`);
      qc.invalidateQueries({ queryKey: ["hal", ch] });
      qc.invalidateQueries({ queryKey: ["backends"] });
      onEngineSwitched?.();
    },
    onError: (e) => toast.err(`互斥启动失败：${e}`),
  });

  // 启动入口：配对后端先查对面引擎是否还在跑（同端口互斥，一家内存只够跑一家）；
  // 在跑 → 弹确认，确认后走互斥切换；没在跑/查不到（对面挂了）→ 普通启动。
  const onStart = async () => {
    if (backend && altEngine) {
      try {
        const other = await ipc<HalStatus>("halctl_status", { ch: altEngine });
        if (other.running) {
          setConfirm({ startExclusive: altEngine });
          return;
        }
      } catch {
        /* 对面状态查不到 = 大概率没在跑，按普通启动走，失败自有报错兜底 */
      }
    }
    control.mutate("start");
  };

  // 版本域：query + 四个 mutation + 下拉口径（见 halogen/useHalVersions）。
  const {
    versions,
    v,
    latest,
    hasUpdate,
    allTags,
    localSet,
    setPickTag,
    chosenTag,
    switchVer,
    runVersionConfirm,
    busyVer,
    busyTextVer,
  } = useHalVersions(ch);

  const busy = control.isPending || exclusiveStart.isPending || busyVer;
  const busyText =
    busyTextVer ||
    (exclusiveStart.isPending
      ? "互斥启动中：先停在跑的引擎再起本引擎，等就绪约数分钟…"
      : control.isPending
        ? "执行中（启停会等待引擎就绪，约 1~5 分钟）…"
        : "");

  const s = status.data;
  const h = (s?.health ?? {}) as Record<string, unknown>;
  const c = (s?.cache ?? {}) as Record<string, unknown>;
  const ver = (h.version ?? {}) as Record<string, unknown>;
  const pool = (c.pool ?? {}) as Record<string, unknown>;
  // metrics 单独 memo：`?? {}` 每次渲染都是新对象，会让下面两个均值 useMemo 失效。
  const m = useMemo(
    () => (s?.metrics ?? {}) as Record<string, number>,
    [s?.metrics],
  );
  const kvRatio = m["llamacpp:kv_cache_usage_ratio"];
  const kvTokens = m["llamacpp:kv_cache_tokens"];

  const prefillAvg = useMemo(() => {
    const tk = m["llamacpp:prompt_tokens_total"];
    const sec = m["llamacpp:prompt_seconds_total"];
    return tk != null && sec ? tk / sec : null;
  }, [m]);
  const decodeAvg = useMemo(() => {
    const tk = m["llamacpp:tokens_predicted_total"];
    const sec = m["llamacpp:tokens_predicted_seconds_total"];
    return tk != null && sec ? tk / sec : null;
  }, [m]);

  // 确认分发：控制域分支在这、版本域分支在 useHalVersions 里。
  const runConfirm = () => {
    if (confirm === "stop") control.mutate("stop");
    else if (confirm === "restart") control.mutate("restart");
    else if (confirm && typeof confirm === "object" && "startExclusive" in confirm)
      exclusiveStart.mutate();
    else runVersionConfirm(confirm);
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-auto pr-0.5">
      {/* 状态 + 控制 */}
      <Card className="space-y-3 py-3">
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
          <span
            className={`inline-block h-2.5 w-2.5 rounded-full ${
              !s?.reachable
                ? "bg-rose-500"
                : s.running
                  ? "bg-emerald-400"
                  : "bg-slate-500"
            }`}
          />
          <span className="text-sm font-medium text-ink-1">
            {!s?.reachable ? "不可达" : s.running ? "运行中" : "已停止"}
          </span>
          <span className="font-mono text-xs text-cyan-300">v{str(ver.api) || "—"}</span>
          <span className="text-xs text-ink-3">{str(h.model) || "—"}</span>
          <span className="ml-auto text-xs text-ink-4">
            {s?.reachable ? `链路 ${s.host === "10.0.0.102" ? "雷电" : "局域网"} · ${s.host}` : "两个地址都连不上"}
          </span>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          {s?.running ? (
            <>
              <Button variant="danger" disabled={busy} onClick={() => setConfirm("stop")}>
                停止
              </Button>
              <Button variant="danger" disabled={busy} onClick={() => setConfirm("restart")}>
                重启
              </Button>
            </>
          ) : (
            <Button variant="primary" disabled={busy} onClick={() => void onStart()}>
              启动
            </Button>
          )}
          {busyText && <span className="text-xs text-[var(--sem-busy-text)]">{busyText}</span>}
          <Button variant={view === "params" ? "primary" : "ghost"} onClick={() => setView(view === "params" ? "logs" : "params")}>
            {view === "params" ? "收起参数" : "参数设置"}
          </Button>
          {(control.data?.ok === false || switchVer.data?.ok === false) && (
            <span className="text-xs text-[var(--sem-danger-text)]">上一步操作返回失败，见日志</span>
          )}
        </div>

        <div className="grid grid-cols-3 gap-x-4 gap-y-2 text-xs sm:grid-cols-6">
          <Stat label="上下文" value={fmt(num(h.context))} />
          <Stat label="并发槽" value={fmt(num(h.slots))} />
          <Stat label="在飞请求" value={fmt(num(h.in_flight))} />
          <Stat label="排队" value={fmt(num(h.queued))} />
          <Stat label="忙碌时长" value={num(h.busy_for_s) != null ? `${num(h.busy_for_s)}s` : "—"} />
          <Stat
            label="KV 池"
            value={
              num(pool.used) != null && num(pool.positions) != null
                ? `${Math.round(((num(pool.used) ?? 0) / (num(pool.positions) ?? 1)) * 100)}%`
                : "—"
            }
          />
        </div>

        {/* KV 实时占用条（承 hal_dash 口径：usage_ratio + kv_cache_tokens，3s 轮询驱动） */}
        {kvRatio != null && (
          <div>
            <div className="flex items-baseline justify-between text-xs text-ink-4">
              <span>KV 池占用</span>
              <span className="font-mono text-ink-3">
                {kvTokens != null ? `${fmt(kvTokens)} token · ` : ""}
                {(kvRatio * 100).toFixed(1)}%
              </span>
            </div>
            <div className="mt-1 h-1.5 w-full overflow-hidden rounded-full bg-slate-800">
              <div
                className={`h-full rounded-full transition-all duration-700 ${
                  kvRatio < 0.7 ? "bg-emerald-400" : kvRatio < 0.9 ? "bg-amber-400" : "bg-rose-500"
                }`}
                style={{ width: `${Math.min(100, Math.max(0, kvRatio * 100))}%` }}
              />
            </div>
          </div>
        )}
      </Card>

      {/* 指标 */}
      <Card className="space-y-2 py-3">
        <div className="text-xs font-medium text-ink-2">运行指标（累计均值）</div>
        <div className="grid grid-cols-2 gap-x-4 gap-y-2 text-xs sm:grid-cols-4">
          <Stat label="预填均速" value={prefillAvg != null ? `${fmt(prefillAvg)} t/s` : "—"} tone="cyan" />
          <Stat label="生成均速" value={decodeAvg != null ? `${fmt(decodeAvg)} t/s` : "—"} tone="cyan" />
          <Stat label="处理中" value={fmt(m["llamacpp:requests_processing"] ?? null)} />
          <Stat label="被搁置" value={fmt(m["llamacpp:requests_deferred"] ?? null)} />
          <Stat label="缓存条目" value={fmt(num(c.entries))} />
          <Stat
            label="缓存命中"
            value={
              num(c.hits) != null && num(c.misses) != null
                ? `${fmt(num(c.hits))} / ${fmt(num(c.misses))}`
                : "—"
            }
          />
          <Stat label="缓存占用" value={fmtBytes(num(c.bytes))} />
          <Stat
            label="MTP 接受率"
            value={
              m["halogen:draft_tokens_accepted_total"] != null && m["halogen:draft_tokens_total"]
                ? `${Math.round((m["halogen:draft_tokens_accepted_total"] / (m["halogen:draft_tokens_total"] || 1)) * 100)}%`
                : "—"
            }
          />
        </div>
      </Card>

      {/* 版本：ghcr 官方列表 60s 自动刷新 + 手动「检查更新」；有新版给徽标和一键更新。 */}
      <Card className="space-y-2 py-3">
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs">
          <span className="font-medium text-ink-2">版本</span>
          <span className="text-ink-3">
            当前 <span className="font-mono text-ink-2">{v?.current || "—"}</span>
          </span>
          <span className="text-ink-3">
            线上最新 <span className="font-mono text-ink-2">{latest || "—"}</span>
          </span>
          {hasUpdate && (
            <span className="rounded-full border border-cyan-500/50 bg-cyan-500/15 px-2 py-0.5 text-2xs text-cyan-200">
              发现新版 {v?.current} → {latest}
            </span>
          )}
          {v?.backup?.ver && (
            <span className="text-ink-3">
              备份 <span className="font-mono text-ink-2">{v.backup.ver}</span>
              <span className="text-ink-4">（{v.backup.when}）</span>
            </span>
          )}
        </div>
        {/* 失败原因必须可见：以前静默显示「—」，用户以为没有检测功能 */}
        {versions.isError && (
          <div className="text-xs text-[var(--sem-danger-text)]">版本信息加载失败：{String(versions.error)}</div>
        )}
        {v?.remote_error && (
          <div className="text-xs text-[var(--sem-busy-text)]">ghcr 官方列表获取失败：{v.remote_error}</div>
        )}
        <div className="flex flex-wrap items-center gap-2">
          {hasUpdate && (
            <Button variant="primary" disabled={busy} onClick={() => setConfirm({ switchTag: latest })}>
              一键更新到最新
            </Button>
          )}
          <select className={fieldCls} value={chosenTag} onChange={(e) => setPickTag(e.target.value)}>
            {allTags.map((t) => (
              <option key={t} value={t}>
                {t}
                {localSet.has(t) ? "（本地）" : ""}
                {t === latest ? "（最新）" : ""}
              </option>
            ))}
          </select>
          <Button
            variant="primary"
            disabled={busy || !chosenTag || chosenTag === v?.current}
            onClick={() => setConfirm({ switchTag: chosenTag })}
          >
            切换/更新到此版本
          </Button>
          {/* 删除选中的本地镜像：与切换版本同一界面，用的就是上面那个版本下拉框。
              只有本地真有该镜像、且它不是当前版本时才给按（当前版本删了等于把在跑的
              引擎底下抽走，102 端也会拒）。 */}
          <Button
            variant="danger"
            disabled={busy || !localSet.has(chosenTag) || chosenTag === v?.current}
            onClick={() => setConfirm({ delTag: chosenTag })}
            title={
              chosenTag === v?.current
                ? "当前版本不许删，先切到别的版本"
                : localSet.has(chosenTag)
                  ? `删除本地镜像 ${chosenTag}（腾磁盘）`
                  : "该版本本地没有镜像，无需删除"
            }
          >
            删除此本地镜像
          </Button>
          <Button disabled={busy} onClick={() => setConfirm("backup")}>
            存当前为备份
          </Button>
          <Button
            disabled={busy || !v?.backup?.ver}
            onClick={() => setConfirm("restore")}
            title={v?.backup?.ver ? `恢复到 ${v.backup.ver}` : "还没有备份槽"}
          >
            恢复备份
          </Button>
          <Button onClick={() => versions.refetch()} title="重新拉取 ghcr 官方版本列表">
            检查更新
          </Button>
        </div>
        {switchVer.data && (
          <pre className="max-h-32 overflow-auto whitespace-pre-wrap rounded-md border border-slate-800 bg-slate-950/60 p-2 text-xs leading-relaxed text-ink-3">
            {switchVer.data.output}
          </pre>
        )}
      </Card>

      {/* 日志/参数共用这一张卡，两种视图的高度策略不同，别混成一条：
          - 日志：吃满剩余视口（flex-1），min-h 保底是因为 flex-1 的 basis 是 0，
            上半部卡片占满时这里会被压到 0 高（用户实报「日志 UI 根本不显示」）。
          - 参数：按内容自然长高、由外层面板统一滚动。这里绝不吃 flex-1，也不拿 vh 兜底：
            缩放挂在 documentElement 的 CSS zoom 上，vh 会被倍率乘高（全站既有约定，
            Backends/ModelParamsDialog 都留了同样的警告），而「被压成一条缝」的真正原因
            是表单自己套了内层滚动区，已在 HalogenParams 里去掉。 */}
      <Card
        className={`flex flex-col space-y-2 py-3 ${
          view === "params" ? "" : "min-h-[260px] flex-1"
        }`}
      >
        {view === "params" ? (
          <HalogenParams ch={ch} onBack={() => setView("logs")} />
        ) : (
          <LogsCard ch={ch} active />
        )}
      </Card>

      <HalogenConfirm
        confirm={confirm}
        backupVer={v?.backup?.ver}
        backupWhen={v?.backup?.when}
        onClose={() => setConfirm(null)}
        onConfirm={runConfirm}
      />
    </div>
  );
}
