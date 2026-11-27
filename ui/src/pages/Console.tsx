// 后端控制台页（M7-B）：左列后端 + 右 iframe 内嵌引擎面板。
// 反代通道 /console/:id/ 由网关注入凭据并剥内嵌拦截头（M7-A）。
// halogen 例外：不走 iframe，右侧换成原生面板（SSH 控制面，见 HalogenPanel）。
// 子件（console/ 目录）：BackendTags 标签网格 / EntrySettings 入口表单 /
// ConsoleFrame iframe 帧 / frameSrc.ts URL 口径 / EngineSwitch / useConsoleFlow。
import { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Button, Card } from "../components/ui";

import { useEscape } from "../components/a11y";
import { t, tf, ts } from "../lib/i18n";
import { useToast } from "../components/Toast";
import { ConfirmDialog, Risk } from "../components/ConfirmDialog";
import {
  ipc,
  type BackendRow,
  type DeleteOutcome,
  type ScriptOutcome,
  type ShellStatus,
} from "../lib/ipc";
import { getTheme, subscribeTheme, type Theme } from "../lib/theme";
import { HalogenPanel } from "./HalogenPanel";
import { EngineSwitch } from "./console/EngineSwitch";
import { useConsoleFlow } from "./console/useConsoleFlow";
import { BackendTags } from "./console/BackendTags";
import { EntrySettings } from "./console/EntrySettings";
import { ConsoleFrame } from "./console/ConsoleFrame";
import { buildConsoleUrl, gatewayOrigin } from "./console/frameSrc";

/** 脚本输出塞进 toast 前先收一收：只留最后一行、最多 120 字（完整输出在日志文件里）。
 *  取末行是因为报错总是最后出现的那句。 */
function lastLine(s: string, max = 120): string {
  const line = s
    .split("\n")
    .map((t) => t.trim())
    .filter(Boolean)
    .pop();
  if (!line) return "";
  return line.length > max ? `${line.slice(0, max)}…` : line;
}

export function ConsolePage({
  pendingBackend,
  toolbarSlot,
}: {
  pendingBackend?: string | null;
  /** 后端页页签行右侧槽位（用户 2026-11-21）：工具条搬过去，全屏时仍渲染在面板内。 */
  toolbarSlot?: HTMLElement | null;
}) {
  const qc = useQueryClient();
  const toast = useToast();
  const backends = useQuery({
    queryKey: ["backends"],
    queryFn: () => ipc<BackendRow[]>("list_backends"),
  });
  const status = useQuery({ queryKey: ["status"], queryFn: () => ipc<ShellStatus>("get_status") });

  const [selected, setSelected] = useState<number | null>(null);
  // 有效/无效分段（默认「有效」，用户 2026-11-19 两次截图拍板：先默认只看能开的，
  // 再把它从标签网格上方移进页头）。页头与网格分居两处，状态只能由这里持有。
  const [editing, setEditing] = useState(false);
  // 配对后端（halogen/gufo）当前在看哪个引擎面板：只切视图、不动引擎
  // （用户口径：点 tab 是切页面；互斥切换发生在面板里按「启动」时）。
  const [chView, setChView] = useState<string | null>(null);
  // 删除二次确认（不可逆动作必弹，同后端页口径）。
  const [deleteConfirm, setDeleteConfirm] = useState(false);

  // 控制台左列点亮：每个后端此刻在飞的请求数（口径见 useConsoleFlow）。
  const { flow, ready } = useConsoleFlow();
  const [fullscreen, setFullscreen] = useState(false);
  // 全屏时 Esc 退出（U10）。这不是模态，所以走冒泡阶段、并且让位给真正打开着的对话框
  // （删除确认那类 Radix 弹窗自己吃 Esc，这里抢不得）。
  useEscape(
    () => {
      if (!document.querySelector('[role="dialog"]')) setFullscreen(false);
    },
    fullscreen,
    false,
  );
  const [frameKey, setFrameKey] = useState(0);

  // 主题跟随：内嵌面板（自家 logview 那类）跟着应用主题走，别在浅色界面里嵌一块死黑。
  const [theme, setTheme] = useState<Theme>(getTheme());
  useEffect(() => {
    const off = subscribeTheme(setTheme);
    return () => {
      off();
    };
  }, []);

  // useMemo 稳定引用：`?? []` 在没数据时每次渲染都是新数组，会让下面那个
  // [rows] 依赖的 effect 每次渲染都重跑（lint 指出的正是这一点）。
  const rows = useMemo(() => backends.data ?? [], [backends.data]);
  const sel = rows.find((b) => b.id === selected) ?? rows[0] ?? null;
  // 进页默认激活「此刻真正在用」的后端（在飞请求 >0），而不是永远落第一行
  // （用户 2026-11-20 拍板）。只自动选一次：用户点过就不抢；首轮流量轮询没回
  // 之前不动手，免得 rows 先到误选 rows[0]。
  const autoPicked = useRef(false);
  useEffect(() => {
    if (autoPicked.current || !ready || !rows.length) return;
    autoPicked.current = true;
    const live = rows.find((b) => (flow[b.name] ?? 0) > 0);
    if (live) setSelected((cur) => cur ?? live.id);
  }, [ready, rows, flow]);
  // halogen/gufo 的控制面是原生面板（状态/指标/日志/启停/版本/参数），不是 iframe。
  // 两个引擎同机互斥、同一套 SSH 通道，只差机位（ch 分派，见 HalogenPanel）。
  const isHalogen =
    (sel?.engine ?? "") === "halogen" || (sel?.engine ?? "") === "gufo";
  // 配对后端在看哪个引擎面板：未手动切视图时跟生效引擎。
  const viewCh = chView ?? sel?.engine ?? "halogen";

  // 其他页「去控制台」跳转预选：每个跳转信号只消费一次，refetch 不抢回手动选择。
  const handled = useRef<string | null>(null);
  useEffect(() => {
    if (!pendingBackend || handled.current === pendingBackend) return;
    const hit = rows.find((b) => b.name === pendingBackend);
    if (hit) {
      setSelected(hit.id);
      handled.current = pendingBackend;
    }
  }, [pendingBackend, rows]);

  // 换后端时视图引擎回到该后端的生效引擎（库存视图不跨后端串门），
  // 编辑态一并收起（原先预填 effect 里顺带做的，表单搬走后留在页层）。
  useEffect(() => {
    setChView(null);
    setEditing(false);
  }, [sel?.id]);

  const gatewayBase = gatewayOrigin(status.data?.listen ?? "");
  // 「新窗口」按钮用：与 iframe 同口径（无尾斜杠 / 令牌 / ?theme=，见 frameSrc.ts）。
  const frameSrc = sel ? buildConsoleUrl(sel, gatewayBase, status.data, theme) : "";

  // 起停脚本（见 services::script）：启动不等它跑完，停止等它收尾。
  const runScript = useMutation({
    mutationFn: (action: "start" | "stop") =>
      ipc<ScriptOutcome>("run_backend_script", { name: sel?.name ?? "", action }),
    onSuccess: (r, action) => {
      const label = action === "start" ? t("cs.start") : t("cs.stop");
      if (!r.ok) {
        toast.err(
          r.exit_code === null
            ? ts("cs.failPlain", { label, msg: lastLine(r.output) || t("cs.noOutput") })
            : ts("cs.failCode", {
                label,
                code: r.exit_code,
                msg: lastLine(r.output) || t("cs.noOutput"),
              }),
        );
        return;
      }
      const tail = lastLine(r.output);
      toast.ok(
        ts("cs.okDone", { label }) + (r.pid ? ts("cs.pidAt", { pid: r.pid }) : "") + (tail ? ts("cs.tailAt", { tail }) : ""),
      );
      // 起停之后健康态与左列状态点必然过时：探活循环会自己纠正，这里重读一次
      // 让状态点与控制台画面都跟上（status 那条只是运行时统计，跟起停无关）。
      qc.invalidateQueries({ queryKey: ["backends"] });
      setFrameKey((k) => k + 1);
    },
    onError: (e, action) =>
      toast.err(
        ts("cs.failPlain", {
          label: action === "start" ? t("cs.start") : t("cs.stop"),
          msg: String(e),
        }),
      ),
  });

  // 删除后端：与后端页同一条 IPC、同一个二次确认口径
  // （破坏性动作闸门矩阵，docs/UI-OPTIMIZATION-PLAN.md §四）。
  const deleteMut = useMutation({
    mutationFn: (name: string) => ipc<DeleteOutcome>("delete_backend", { name }),
    onSuccess: (r, name) => {
      toast.ok(
        ts("cs.deleted", { name }) +
          (r.dropped_params || r.dropped_meta
            ? ts("cs.droppedClean", { p: r.dropped_params, m: r.dropped_meta })
            : ""),
      );
      setSelected(null);
      setEditing(false);
      qc.invalidateQueries({ queryKey: ["backends"] });
    },
    onError: (e) => toast.err(ts("cs.deleteFail", { e: String(e) })),
  });

  // 工具条（用户 2026-11-21 拍板并进页签行）：常态 portal 进页签行右侧槽位；
  // 全屏时页签行被遮罩盖住，工具条必须留在面板内，否则退不出去。
  const toolbar = (
    <>
      <span className="text-sm font-medium text-ink-2">{sel?.name ?? t("cs.pickBackend")}</span>
      {sel && (
        <span className="truncate font-mono text-xs text-ink-4" title={sel?.webui_url || `${sel?.base_url ?? ""}${ts("cs.fallback", { u: "" })}`}>
          {isHalogen ? t("cs.sshPanel") : sel.webui_url || ts("cs.fallback", { u: sel.base_url })}
        </span>
      )}
      {/* 引擎面板视图切换（配对后端才出现）：只切右侧面板看谁，不动引擎；
          互斥切换（先停在跑的再起目标）在面板里按「启动」时才确认执行。 */}
      {isHalogen && sel?.alt_engine && (
        <EngineSwitch engine={sel.engine ?? "halogen"} alt={sel.alt_engine} view={viewCh} onView={setChView} />
      )}
      <span className="ml-auto flex items-center gap-2">
        <Button onClick={() => setFrameKey((k) => k + 1)} disabled={!sel}>
          {t("cs.refresh")}
        </Button>
        {!isHalogen && (
          <>
            <Button
              title={
                sel?.start_script
                  ? ts("cs.execScript", { script: sel.start_script })
                  : t("cs.noStartScript")
              }
              disabled={!sel?.start_script || runScript.isPending}
              onClick={() => runScript.mutate("start")}
            >
              {runScript.isPending && runScript.variables === "start" ? t("cs.starting") : t("cs.start")}
            </Button>
            <Button
              title={
                sel?.stop_script
                  ? ts("cs.execScript", { script: sel.stop_script })
                  : t("cs.noStopScript")
              }
              disabled={!sel?.stop_script || runScript.isPending}
              onClick={() => runScript.mutate("stop")}
            >
              {runScript.isPending && runScript.variables === "stop" ? t("cs.stopping") : t("cs.stop")}
            </Button>
            <Button onClick={() => setEditing((v) => !v)} disabled={!sel}>
              {editing ? t("cs.collapseEntry") : t("cs.entrySettings")}
            </Button>
            <Button onClick={() => window.open(frameSrc, "_blank")} disabled={!frameSrc}>
              {t("cs.newWindow")}
            </Button>
            <Button onClick={() => setFullscreen((v) => !v)} disabled={!sel}>
              {fullscreen ? t("cs.exitFull") : t("cs.fullscreen")}
            </Button>
            <Button
              variant="danger"
              disabled={!sel}
              title={t("cs.delBackendTip")}
              onClick={() => setDeleteConfirm(true)}
            >
              {t("common.delete")}
            </Button>
          </>
        )}
      </span>
    </>
  );

  return (
    <div className="flex h-full flex-col gap-3 pt-2">
      {/* 标签行吸顶（用户 2026-11-20 拍板）：滚动时后端切换条不动，下面的面板从它底下滑过。
          -mb-3 抵掉 gap、pb-3 把这段缝收进吸顶背景里，滚动内容不会从缝里露出来。 */}
      <div className="sticky top-0 z-10 -mb-3 bg-slate-950 pb-3">
        <BackendTags rows={rows} selected={selected} flow={flow} onSelect={setSelected} />
      </div>

      {/* 下方：工具条 + iframe（占满整行宽度） */}
      <div
        className={`flex min-w-0 flex-1 flex-col ${
          fullscreen ? "fixed inset-0 z-50 bg-slate-950 p-3" : ""
        }`}
      >
        {fullscreen ? (
          <Card className="mb-3 flex flex-wrap items-center gap-2 py-2">{toolbar}</Card>
        ) : (
          toolbarSlot && createPortal(<>{toolbar}</>, toolbarSlot)
        )}

        {sel && !isHalogen && (
          <EntrySettings
            sel={sel}
            open={editing}
            onClose={() => setEditing(false)}
            onSaved={() => {
              qc.invalidateQueries({ queryKey: ["backends"] });
              setFrameKey((k) => k + 1);
            }}
          />
        )}

        {sel ? (
          isHalogen ? (
            // key=frameKey+viewCh：工具条「刷新」重挂载面板，与 iframe 刷新同语义；
            // 切视图引擎也重挂载，让 ch 相关的本地态（日志档位等）干净重置。
            <HalogenPanel
              key={`${frameKey}:${viewCh}`}
              ch={viewCh}
              backend={sel?.alt_engine ? sel.name : undefined}
              altEngine={sel?.alt_engine ? (viewCh === "halogen" ? "gufo" : "halogen") : undefined}
              onEngineSwitched={() => qc.invalidateQueries({ queryKey: ["backends"] })}
            />
          ) : (
            <ConsoleFrame
              sel={sel}
              gatewayBase={gatewayBase}
              token={status.data?.access_token}
              theme={theme}
              frameKey={frameKey}
              onReload={() => setFrameKey((k) => k + 1)}
              onEdit={() => setEditing(true)}
            />
          )
        ) : (
          <Card className="flex flex-1 items-center justify-center text-sm text-ink-4">
            {t("cs.pickHint")}
          </Card>
        )}
      </div>

      {deleteConfirm && sel && (
        <ConfirmDialog
          open
          danger
          title={ts("cs.delTitle", { name: sel.name })}
          irreversible
          onClose={() => setDeleteConfirm(false)}
          onConfirm={() => deleteMut.mutate(sel.name)}
          confirmLabel={t("cs.delConfirm")}
        >
          {t("cs.delBody1")}
          <br />
          {tf("cs.delBody2", { risk: <Risk>{t("cs.delRisk")}</Risk> })}
          <br />
          <span className="text-ink-4">
            {ts("ag.modelsN", { n: sel.models.length })}
            {sel.api_key ? t("cs.delKeyNote") : ""}
          </span>
        </ConfirmDialog>
      )}
    </div>
  );
}
