// 更新面板（fix:update-check-and-install，用户 2026-11-17：「点击可以下载编译的安装镜像，
// 下载后自动安装更新」，自动安装按拍板的 A 档：下载 → SHA256 校验 → 拉起系统安装器）。
//
// 三条硬口径：
// 1. 没有 SHA256 的包**不给自动安装按钮**（后端也会拒），只能去发布页自己下——不许装没校验过的包；
// 2. macOS 拉起的是 .dmg，**仍需用户把 App 拖进 Applications**，文案必须说清，
//    不许写得像已经装好了；
// 3. 检测失败时面板里如实说失败原因，不假装「已是最新」。
import { useEffect, useState } from "react";
import * as Dialog from "@radix-ui/react-dialog";
import { useQuery } from "@tanstack/react-query";
import { Button } from "./ui";
import { useToast } from "./Toast";
import { Risk } from "./ConfirmDialog";
import { ipc } from "../lib/ipc";
import { DEMO_VIDEO, openExternal } from "../lib/links";
import { t } from "../lib/i18n";
import { every, POLL } from "../lib/poll";
import {
  launchUpdateInstall,
  startUpdateDownload,
  useRefreshUpdate,
  RELEASE_PAGES,
  type UpdateAsset,
  type UpdateInfo,
} from "../lib/update";

interface Task {
  id: string;
  kind: string;
  label: string;
  status: "running" | "done" | "failed";
  progress: number;
  message: string;
}

export function UpdateDialog({
  info,
  onClose,
}: {
  info: UpdateInfo | undefined;
  onClose: () => void;
}) {
  const toast = useToast();
  const refresh = useRefreshUpdate();
  const [taskId, setTaskId] = useState<string | null>(null);
  const [picked, setPicked] = useState<UpdateAsset | null>(null);
  const [checking, setChecking] = useState(false);

  // 只在有下载任务时轮询任务表（与安装引导同一套口径，装完即停）。
  const tasks = useQuery({
    queryKey: ["tasks"],
    queryFn: () => ipc<Task[]>("list_tasks"),
    refetchInterval: () => (taskId ? every(POLL.fast) : false),
  });
  const task = taskId ? tasks.data?.find((x) => x.id === taskId) : undefined;
  const running = task?.status === "running";

  // 下载完成 → 自动拉起安装器（A 档）；失败就把原因摊开。
  useEffect(() => {
    if (!task || task.status === "running") return;
    if (task.status === "failed") {
      toast.err(`${t("upd.dlFail")}：${task.message}`);
      setTaskId(null);
      return;
    }
    const path = task.message;
    launchUpdateInstall(path)
      .then((msg) => {
        toast.ok(msg);
        onClose();
      })
      .catch((e) => toast.err(`${t("upd.launchFail")}：${e}`))
      .finally(() => setTaskId(null));
    // 依赖只认任务状态：路径在 message 里，别把它塞进依赖导致重复拉起。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [task?.status]);

  const manualCheck = async () => {
    setChecking(true);
    try {
      const d = await refresh();
      if (d.error) toast.err(`${t("upd.checkFail")}：${d.error}`);
      else if (d.has_update) toast.ok(`${t("upd.found")} ${d.version}`);
      else toast.ok(`${t("upd.latest")} ${d.current}`);
    } catch (e) {
      toast.err(`${t("upd.checkFail")}：${e}`);
    } finally {
      setChecking(false);
    }
  };

  const start = (a: UpdateAsset) => {
    if (!a.sha256) return; // 没校验和的包不给装（按钮也已禁用，这里兜一层）
    setPicked(a);
    startUpdateDownload(a)
      .then(setTaskId)
      .catch((e) => toast.err(`${t("upd.dlFail")}：${e}`));
  };

  return (
    <Dialog.Root open onOpenChange={(o) => !o && onClose()}>
      <Dialog.Portal>
        {/* z-[90] 与命令面板同档：Radix Portal 默认无 z-index，会被 sticky 表头（z-10）穿透。 */}
        <Dialog.Overlay className="fixed inset-0 z-[90] bg-black/60" />
        <Dialog.Content className="fixed left-1/2 top-1/2 z-[90] max-h-[86%] w-[560px] max-w-[94vw] -translate-x-1/2 -translate-y-1/2 overflow-y-auto rounded-xl border border-slate-700 bg-slate-900 p-5 shadow-2xl">
          {/* 标题栏带讲解视频地址（用户 2026-11-19）：裸地址直接摆出来、点击走
              应用内浏览器（lib/links.ts 三级兜底）——与底部两个公仓地址同一条口径。 */}
          <Dialog.Title className="flex flex-wrap items-baseline gap-x-3 gap-y-1 text-base font-semibold text-ink-1">
            {t("upd.title")}
            <a
              className="min-w-0 break-all text-xs font-normal text-cyan-300 hover:underline"
              href={DEMO_VIDEO}
              onClick={(e) => {
                e.preventDefault();
                openExternal(DEMO_VIDEO);
              }}
            >
              {DEMO_VIDEO}
            </a>
          </Dialog.Title>
          <Dialog.Description className="mt-0.5 text-xs text-ink-4">
            {info
              ? `${t("upd.current")} ${info.current} · ${t("upd.latestVer")} ${info.version || "—"} · ${info.source || "—"}`
              : t("upd.reading")}
          </Dialog.Description>

          {info?.error && (
            <p className="mt-2 rounded border border-[var(--sem-danger-text)]/40 bg-[var(--sem-danger-text)]/10 px-2 py-1.5 text-xs text-[var(--sem-danger-text)]">
              {t("upd.checkFail")}：{info.error}
            </p>
          )}
          {info && !info.has_update && !info.error && (
            <p className="mt-3 text-xs text-ink-3">{t("upd.noUpdate")}</p>
          )}

          {info?.has_update && (
            <>
              <p className="mt-3 text-xs text-ink-2">
                {t("upd.found")} <b className="font-mono">{info.version}</b>
                {info.published ? ` · ${info.published.slice(0, 10)}` : ""}
              </p>
              {info.notes && (
                <details className="mt-2">
                  <summary className="cursor-pointer text-xs text-ink-3 hover:text-ink-1">
                    {t("upd.notes")}
                  </summary>
                  <pre className="mt-1 max-h-52 overflow-auto whitespace-pre-wrap break-all rounded bg-slate-950/60 p-2 font-mono text-2xs text-ink-3">
                    {info.notes}
                  </pre>
                </details>
              )}

              <p className="mt-3 text-xs font-medium text-ink-2">{t("upd.packages")}</p>
              <ul className="mt-1 space-y-1.5">
                {info.assets.map((a) => (
                  <li
                    key={a.name}
                    className={`rounded border px-2 py-1.5 text-xs ${
                      a.platform_match
                        ? "border-cyan-500/50 bg-cyan-500/10"
                        : "border-slate-700 bg-slate-800/40"
                    }`}
                  >
                    <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
                      <span className="min-w-0 break-all font-mono text-ink-1">{a.name}</span>
                      {a.platform_match && (
                        <span className="shrink-0 rounded bg-cyan-500/20 px-1.5 py-px text-2xs text-cyan-200">
                          {t("upd.thisPlatform")}
                        </span>
                      )}
                      <span className="ml-auto flex shrink-0 items-center gap-1.5">
                        {a.url && (
                          <a
                            className="text-cyan-300 hover:underline"
                            href={a.url}
                            onClick={(e) => {
                              e.preventDefault();
                              openExternal(a.url!);
                            }}
                          >
                            GitHub
                          </a>
                        )}
                        {a.gitee_url ? (
                          <a
                            className="text-cyan-300 hover:underline"
                            href={a.gitee_url}
                            onClick={(e) => {
                              e.preventDefault();
                              openExternal(a.gitee_url!);
                            }}
                          >
                            Gitee
                          </a>
                        ) : (
                          <span className="text-ink-4" title={t("upd.noGitee")}>
                            Gitee —
                          </span>
                        )}
                        <Button
                          size="xs"
                          variant="primary"
                          disabled={running || !a.sha256}
                          title={a.sha256 ? t("upd.dlTip") : t("upd.noSha")}
                          onClick={() => start(a)}
                        >
                          {running && picked?.name === a.name ? t("upd.downloading") : t("upd.dlInstall")}
                        </Button>
                      </span>
                    </div>
                    <div className="mt-0.5 truncate font-mono text-2xs text-ink-4" title={a.sha256 ?? ""}>
                      {a.sha256 ? `SHA256 ${a.sha256}` : t("upd.noSha")}
                    </div>
                  </li>
                ))}
              </ul>

              {running && (
                <div className="mt-2">
                  <div className="h-1 w-full overflow-hidden rounded bg-slate-700">
                    <div
                      className="h-full bg-cyan-400 transition-all"
                      style={{ width: `${Math.max(4, Math.round((task?.progress ?? 0) * 100))}%` }}
                    />
                  </div>
                  <div className="mt-1 truncate text-2xs text-ink-4" title={task?.message}>
                    {task?.message}
                  </div>
                </div>
              )}

              <p className="mt-3 text-2xs text-ink-4">
                <Risk>{t("upd.macNote")}</Risk>
              </p>
            </>
          )}

          {/* 底部两个公仓入口（用户 2026-11-19：不要「打开发布页 / Gitee 发布页」
              这类标签文字，直接把实际地址摆出来，点不点由人看地址自己判断）。
              flex-wrap + break-all：两条长地址在窄对话框里掉行不挤爆。 */}
          <div className="mt-4 flex flex-wrap items-center gap-2">
            <Button size="sm" variant="ghost" disabled={checking} onClick={manualCheck}>
              {checking ? t("upd.checking") : t("upd.recheck")}
            </Button>
            <a
              className="min-w-0 break-all text-xs text-cyan-300 hover:underline"
              href={info?.page_url || RELEASE_PAGES.github}
              onClick={(e) => {
                e.preventDefault();
                openExternal(info?.page_url || RELEASE_PAGES.github);
              }}
            >
              {info?.page_url || RELEASE_PAGES.github}
            </a>
            <a
              className="min-w-0 break-all text-xs text-cyan-300 hover:underline"
              href={RELEASE_PAGES.gitee}
              onClick={(e) => {
                e.preventDefault();
                openExternal(RELEASE_PAGES.gitee);
              }}
            >
              {RELEASE_PAGES.gitee}
            </a>
            <Button size="sm" className="ml-auto" onClick={onClose}>
              {t("upd.close")}
            </Button>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
