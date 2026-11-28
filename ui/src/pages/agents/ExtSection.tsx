// 扩展节（各客户端共用）：三态列表 + 一键安装/更新 + 一键卸载，装清单与口径说明由后端按 app 给。
//
// 八家装的是同一个「速率扩展」：数值一律取网关 `/admin/rates/line?app=<代理>`，与浮窗
// 单行速率线同源同口径（引擎原生速率优先 → 退回字节 × 纠偏系数的校准值）。差别只在挂载
// 方式——有实时展示位的走状态线（claude/codebuddy）、TUI 插件（opencode/deveco）、扩展
// widget（pi）；没有的（codex/gemini/copilot）走官方技能或扩展命令。逐家机制查证见
// `src-tauri/src/services/ext_install.rs` 模块头。
//
// 速率状态线（RateLineSection）是另一条路：llm-switch 自己起的独立小窗，不往各家配置目录
// 写任何东西，所有代理都有。两者数值同源，选一个看即可。
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useToast } from "../../components/Toast";
import { Button, Toggle } from "../../components/ui";
import { QueryError } from "../../components/QueryError";
import { ipc } from "../../lib/ipc";
import { t, ts } from "../../lib/i18n";

/** 扩展状态徽标，各客户端共用（色板与 ACTUAL_STYLE 同一套）。ponytail 节共用，不另抄一份色板。 */
export const EXT_STATE_STYLE: Record<string, string> = {
  installed: "border-emerald-500/40 bg-emerald-500/10 text-[var(--sem-ok-text)]",
  outdated: "border-amber-500/40 bg-amber-500/10 text-[var(--sem-busy-text)]",
  missing: "border-slate-600 bg-slate-800/60 text-ink-3",
};
export const EXT_STATE_LABEL: Record<string, string> = {
  installed: "ag.ext.state.installed",
  outdated: "ag.ext.state.outdated",
  missing: "ag.ext.state.missing",
};

export interface ExtRow {
  name: string;
  desc: string;
  state: "installed" | "missing" | "outdated";
  path: string;
}

/** 后端 `ext_status` 的返回：逐项状态 + 这一端机制的人话说明。 */
export interface ExtStatus {
  app: string;
  rows: ExtRow[];
  hint: string;
}

export interface ExtResult {
  name: string;
  action: "installed" | "updated" | "unchanged" | "removed";
  backup: string | null;
}

/** 扩展节的客户端口径：装进哪、装完怎么生效（状态与安装语义后端按 app 分家）。文案存 key。 */
export const EXT_CLIENTS: Record<string, { label: string; hint: string; done: string }> = {
  pi: {
    label: "ag.ext.pi.label",
    hint: "ag.ext.pi.hint",
    done: "ag.ext.pi.done",
  },
  claude: {
    label: "ag.ext.claude.label",
    hint: "ag.ext.claude.hint",
    done: "ag.ext.claude.done",
  },
  codebuddy: {
    label: "ag.ext.codebuddy.label",
    hint: "ag.ext.codebuddy.hint",
    done: "ag.ext.codebuddy.done",
  },
  opencode: {
    label: "ag.ext.opencode.label",
    hint: "ag.ext.opencode.hint",
    done: "ag.ext.opencode.done",
  },
  deveco: {
    label: "ag.ext.deveco.label",
    hint: "ag.ext.deveco.hint",
    done: "ag.ext.deveco.done",
  },
  codex: {
    label: "ag.ext.codex.label",
    hint: "ag.ext.codex.hint",
    done: "ag.ext.codex.done",
  },
  gemini: {
    label: "ag.ext.gemini.label",
    hint: "ag.ext.gemini.hint",
    done: "ag.ext.gemini.done",
  },
  copilot: {
    label: "ag.ext.copilot.label",
    hint: "ag.ext.copilot.hint",
    done: "ag.ext.copilot.done",
  },
};

/** 速率状态线开关：一个代理一条线、各自独立（同时开 pi 和 codex 是常态），
 * 每条线一个窗口，用户拖到哪就记在哪。 */
export function RateLineSection({ app }: { app: string }) {
  const qc = useQueryClient();
  const toast = useToast();
  const lines = useQuery({
    queryKey: ["rateline"],
    queryFn: () => ipc<string[]>("rateline_list"),
  });
  const toggle = useMutation({
    mutationFn: (on: boolean) => ipc<{ ok: boolean }>("rateline_set", { target: app, on }),
    onSuccess: (r, on) => {
      qc.invalidateQueries({ queryKey: ["rateline"] });
      if (!r.ok) toast.err(on ? t("ag.rateline.createFail") : t("ag.rateline.closeFail"));
      else toast.ok(on ? ts("ag.rateline.opened", { app }) : ts("ag.rateline.closed", { app }));
    },
    onError: (e) => toast.err(ts("ag.rateline.toggleFail", { e: String(e) })),
  });
  const mine = Boolean(lines.data?.includes(app));
  const others = (lines.data ?? []).filter((a) => a !== app);
  return (
    <div className="mt-3 border-t border-slate-800 pt-2">
      <div className="mb-1.5 flex items-center justify-between">
        <span className="text-sm font-medium text-ink-2">{t("ag.rateline.title")}</span>
      </div>
      {lines.isError ? (
        <QueryError inline title={t("ag.rateline.readFail")} error={lines.error} onRetry={() => lines.refetch()} />
      ) : (
        <div className="flex items-center gap-2">
          <span className="text-sm text-ink-3">
            {/* 说的是「清单里有没有它」，不是「此刻看不看得见」——托盘总开关
                能把所有线一起收起来，那时这里依然该是已开启。 */}
            {mine ? t("ag.rateline.on") : t("ag.rateline.off")}
            {others.length > 0 && ts("ag.rateline.others", { apps: others.join("、") })}
          </span>
          <Toggle
            checked={mine}
            disabled={toggle.isPending}
            onChange={(v) => toggle.mutate(v)}
          />
        </div>
      )}
    </div>
  );
}

/** 扩展节（各客户端共用）：三态列表 + 一键安装/更新 + 一键卸载。
 * 卸载只拆我们自己装的东西——状态线被用户改去别处、配置文件里别人的插件条目，一律不碰。
 * framed：与 ponytail 节并排成两张卡时（用户 2026-11-23），去分隔线改卡片外框，
 * 由外层 cards-2 栅格控列；默认仍是详情卡内的横分隔节形态。 */
export function ExtSection({ app, framed = false }: { app: string; framed?: boolean }) {
  const qc = useQueryClient();
  const toast = useToast();
  const meta = EXT_CLIENTS[app];
  const ext = useQuery({
    queryKey: ["ext", app],
    queryFn: () => ipc<ExtStatus>("ext_status", { app }),
  });
  const rows = ext.data?.rows ?? [];
  const installed = rows.some((r) => r.state === "installed" || r.state === "outdated");
  const mutate = useMutation({
    mutationFn: (cmd: "ext_install" | "ext_uninstall") => ipc<ExtResult[]>(cmd, { app }),
    onSuccess: (res, cmd) => {
      qc.invalidateQueries({ queryKey: ["ext", app] });
      const changed = res.filter((r) => r.action !== "unchanged");
      if (changed.length === 0) {
        toast.ok(cmd === "ext_uninstall" ? t("ag.noOurs") : t("ag.alreadyLatest"));
        return;
      }
      // 备份路径只挑一条报——全报一遍会把提示刷成日志。
      const bak = changed.find((r) => r.backup)?.backup;
      toast.ok(
        (cmd === "ext_uninstall"
          ? ts("ag.uninstalledN", { n: changed.length })
          : ts("ag.installedN", { n: changed.length })) +
          " · " +
          (cmd === "ext_uninstall" ? t("ag.origBackedUp") : t(meta?.done ?? "")) +
          (bak ? ts("ag.backupAt", { bak }) : ""),
      );
    },
    onError: (e: unknown, cmd: "ext_install" | "ext_uninstall") =>
      toast.err(
        ts("ag.extOpFail", {
          op: cmd === "ext_uninstall" ? t("ag.opUninstall") : t("ag.opInstall"),
          e: String(e),
        }),
      ),
  });
  if (!meta) return null;
  return (
    <div className={framed ? "rounded-lg border border-slate-800 bg-slate-900/40 p-3" : "mt-3 border-t border-slate-800 pt-2"}>
      {/* ui_04：长说明不再与标题同排右挂（会被挤成一行读不完），改挂标题下方。 */}
      <div className="mb-1.5">
        <span className="text-sm font-medium text-ink-2">{t(meta.label)}</span>
        <div className="text-2xs text-ink-4">{t(meta.hint)}</div>
      </div>
      {ext.isError ? (
        <QueryError inline title={t("ag.extReadFail")} error={ext.error} onRetry={() => ext.refetch()} />
      ) : !ext.data ? (
        <p className="text-sm text-ink-4">{t("common.loading")}</p>
      ) : (
        <div className="space-y-1">
          {rows.map((e) => (
            <div
              key={e.name}
              className="flex items-center gap-2 rounded px-1.5 py-1 hover:bg-slate-800/40"
            >
              {/* 名字可截断（title 看全名），徽标 shrink-0 保住右缘位置：
                  两个都不许缩会让长名字把徽标顶出卡片（fix:ext-section-buttons）。 */}
              <span className="min-w-0 truncate text-sm text-ink-2" title={e.name}>
                {e.name}
              </span>
              <span
                className={`ml-auto shrink-0 rounded border px-1 text-2xs ${EXT_STATE_STYLE[e.state]}`}
                title={e.path}
              >
                {t(EXT_STATE_LABEL[e.state])}
              </span>
            </div>
          ))}
          {/* 按钮行收窄（fix:ext-section-buttons 1 号）：xs 尺寸 + 紧 gap，
              不再占掉整行宽度，右侧留给状态徽标那一列。 */}
          <div className="flex items-center gap-1.5 pt-0.5">
            <Button
              size="xs"
              variant="primary"
              disabled={mutate.isPending}
              onClick={() => mutate.mutate("ext_install")}
            >
              {mutate.isPending && mutate.variables === "ext_install"
                ? t("pt.busy")
                : installed
                  ? t("ag.updateAllBtn")
                  : t("pt.install")}
            </Button>
            {installed && (
              <Button
                size="xs"
                variant="ghost"
                disabled={mutate.isPending}
                title={t("ag.uninstallTip")}
                onClick={() => mutate.mutate("ext_uninstall")}
              >
                {mutate.isPending && mutate.variables === "ext_uninstall" ? t("ag.uninstalling") : t("pt.uninstall")}
              </Button>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
