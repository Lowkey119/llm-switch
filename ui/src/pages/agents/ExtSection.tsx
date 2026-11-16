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

/** 扩展状态徽标，各客户端共用（色板与 ACTUAL_STYLE 同一套）。 */
const EXT_STATE_STYLE: Record<string, string> = {
  installed: "border-emerald-500/40 bg-emerald-500/10 text-[var(--sem-ok-text)]",
  outdated: "border-amber-500/40 bg-amber-500/10 text-[var(--sem-busy-text)]",
  missing: "border-slate-600 bg-slate-800/60 text-ink-3",
};
const EXT_STATE_LABEL: Record<string, string> = {
  installed: "已安装",
  outdated: "有更新",
  missing: "未安装",
};

interface ExtRow {
  name: string;
  desc: string;
  state: "installed" | "missing" | "outdated";
  path: string;
}

/** 后端 `ext_status` 的返回：逐项状态 + 这一端机制的人话说明。 */
interface ExtStatus {
  app: string;
  rows: ExtRow[];
  hint: string;
}

interface ExtResult {
  name: string;
  action: "installed" | "updated" | "unchanged" | "removed";
  backup: string | null;
}

/** 扩展节的客户端口径：装进哪、装完怎么生效（状态与安装语义后端按 app 分家）。 */
export const EXT_CLIENTS: Record<string, { label: string; hint: string; done: string }> = {
  pi: {
    label: "pi 扩展",
    hint: "写入 pi 用户扩展目录 · 重启 pi 生效",
    done: "重启 pi 生效",
  },
  claude: {
    label: "Claude 扩展",
    hint: "写入 Claude Code 配置目录 · 状态栏脚本落盘即生效",
    done: "落盘即生效（refreshInterval=1）",
  },
  codebuddy: {
    label: "CodeBuddy 扩展",
    hint: "写入 CodeBuddy 配置目录 · 状态栏与 Claude 同一套协议",
    done: "落盘即生效（refreshInterval=1）",
  },
  opencode: {
    label: "OpenCode 扩展",
    hint: "写 TUI 插件（V2 自动发现，无需登记）· 重启 opencode 生效",
    done: "重启 opencode 生效",
  },
  deveco: {
    label: "DevEco Code 扩展",
    hint: "写 TUI 插件并登记 deveco.jsonc · 重启 deveco 生效",
    done: "重启 deveco 生效",
  },
  codex: {
    label: "Codex 扩展",
    hint: "写脚本 + 个人技能 · 自定义状态线仍是实验特性",
    done: "问它「现在多快」即报数",
  },
  gemini: {
    label: "Gemini 扩展",
    hint: "gemini extensions install · 扩展面没有实时展示位",
    done: "在 gemini 里 /extensions reload 后用 /rate",
  },
  copilot: {
    label: "Copilot 扩展",
    hint: "copilot plugin install · 插件面没有实时展示位",
    done: "问它「现在多快」即报数",
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
      if (!r.ok) toast.err(on ? "速率线窗口创建失败，看运行日志" : "速率线关闭失败");
      else toast.ok(on ? `已开 ${app} 速率线 · 拖动可换位置` : `已关 ${app} 速率线`);
    },
    onError: (e) => toast.err(`速率线开关失败：${e}`),
  });
  const mine = Boolean(lines.data?.includes(app));
  const others = (lines.data ?? []).filter((a) => a !== app);
  return (
    <div className="mt-3 border-t border-slate-800 pt-2">
      <div className="mb-1.5 flex items-center justify-between">
        <span className="text-xs font-medium text-ink-2">速率状态线</span>
      </div>
      {lines.isError ? (
        <QueryError inline title="读取速率线状态失败" error={lines.error} onRetry={() => lines.refetch()} />
      ) : (
        <div className="flex items-center gap-2">
          <span className="text-xs text-ink-3">
            {/* 说的是「清单里有没有它」，不是「此刻看不看得见」——托盘总开关
                能把所有线一起收起来，那时这里依然该是已开启。 */}
            {mine ? "已开启" : "未开启"}
            {others.length > 0 && ` · 另有 ${others.join("、")}`}
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
 * 卸载只拆我们自己装的东西——状态线被用户改去别处、配置文件里别人的插件条目，一律不碰。 */
export function ExtSection({ app }: { app: string }) {
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
        toast.ok(cmd === "ext_uninstall" ? "没有我们装过的扩展" : "扩展已是最新版本");
        return;
      }
      // 备份路径只挑一条报——全报一遍会把提示刷成日志。
      const bak = changed.find((r) => r.backup)?.backup;
      toast.ok(
        `${cmd === "ext_uninstall" ? "已卸载" : "已安装"} ${changed.length} 项 · ${
          cmd === "ext_uninstall" ? "原件已备份" : (meta?.done ?? "")
        }${bak ? ` · 备份 ${bak}` : ""}`,
      );
    },
    onError: (e: unknown, cmd: "ext_install" | "ext_uninstall") =>
      toast.err(`扩展${cmd === "ext_uninstall" ? "卸载" : "安装"}失败：${e}`),
  });
  if (!meta) return null;
  return (
    <div className="mt-3 border-t border-slate-800 pt-2">
      <div className="mb-1.5 flex items-center justify-between">
        <span className="text-xs font-medium text-ink-2">{meta.label}</span>
        <span className="text-xs text-ink-4">{meta.hint}</span>
      </div>
      {ext.isError ? (
        <QueryError inline title="读取扩展状态失败" error={ext.error} onRetry={() => ext.refetch()} />
      ) : !ext.data ? (
        <p className="text-xs text-ink-4">载入中…</p>
      ) : (
        <div className="space-y-1">
          {rows.map((e) => (
            <div
              key={e.name}
              className="flex items-center gap-2 rounded px-1.5 py-1 hover:bg-slate-800/40"
            >
              <span className="shrink-0 text-xs text-ink-2">{e.name}</span>
              <span
                className={`ml-auto shrink-0 rounded border px-1 text-2xs ${EXT_STATE_STYLE[e.state]}`}
                title={e.path}
              >
                {EXT_STATE_LABEL[e.state]}
              </span>
            </div>
          ))}
          <div className="flex items-center gap-2 pt-0.5">
            <Button
              variant="primary"
              disabled={mutate.isPending}
              onClick={() => mutate.mutate("ext_install")}
            >
              {mutate.isPending && mutate.variables === "ext_install"
                ? "处理中…"
                : installed
                  ? "一键更新"
                  : "一键安装"}
            </Button>
            {installed && (
              <Button
                variant="ghost"
                disabled={mutate.isPending}
                title="只拆我们自己装的：状态线被你改去别处、配置文件里别人的插件条目都不动"
                onClick={() => mutate.mutate("ext_uninstall")}
              >
                {mutate.isPending && mutate.variables === "ext_uninstall" ? "卸载中…" : "卸载"}
              </Button>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
