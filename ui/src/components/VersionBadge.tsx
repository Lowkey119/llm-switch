// 版本相关的共用弹层与顶栏版本徽章。
//
// 这里原先住着顶栏的版本号徽章 VersionChip（UI-AUDIT-2 F-1）与「有更新」呼吸徽标；
// 用户 2026-11-19 拍板把版本入口搬进边条底部，2026-11-20 又拍板搬回顶栏（页名右侧）：
// 点击弹更新来源对话框，检测到新版本时版本号与「有更新」呼吸交替。
// ReposDialog：不管有没有更新，点开都能去双公仓看发布页。
import { useState } from "react";
import * as Dialog from "@radix-ui/react-dialog";
import { RELEASE_PAGES, prefersReducedMotion, useUpdateCheck } from "../lib/update";
import { formatVersion, useAppVersion } from "../lib/version";
import { Button } from "./ui";
import { CopyButton } from "./CopyButton";
import { UpdateDialog } from "./UpdateDialog";
import { openExternal } from "../lib/links";
import { t } from "../lib/i18n";

/** 顶栏版本徽章（用户 2026-11-20 拍板搬回顶栏）：无更新显示版本号、点开双公仓发布页；
 *  检测到新版本时版本号与「有更新」呼吸交替，点开更新弹层直接下载。 */
export function TopVersionChip() {
  const v = formatVersion(useAppVersion());
  const upd = useUpdateCheck();
  const [repos, setRepos] = useState(false);
  const [update, setUpdate] = useState(false);
  const info = upd.data;
  const has = !!info?.has_update;
  return (
    <>
      <button
        type="button"
        onClick={() => (has ? setUpdate(true) : setRepos(true))}
        aria-label={`${t("rail.version")} ${v}`}
        className="shrink-0 rounded px-1 py-0.5 font-mono text-2xs text-ink-4 transition-colors hover:text-ink-1"
      >
        {has && !prefersReducedMotion() ? (
          <span className="ver-alt">
            <span className="ver-alt-a">{v}</span>
            <span className="ver-alt-b text-[var(--sem-busy-text)]">{t("upd.badge")}</span>
          </span>
        ) : has ? (
          <span className="text-[var(--sem-busy-text)]">{t("upd.badge")}</span>
        ) : (
          v
        )}
      </button>
      {repos && <ReposDialog onClose={() => setRepos(false)} />}
      {update && info && <UpdateDialog info={info} onClose={() => setUpdate(false)} />}
    </>
  );
}

/** 双公仓发布页弹层：安装包同步发布在两个公开仓库，点任一个去下载。 */
export function ReposDialog({ onClose }: { onClose: () => void }) {
  const links = [RELEASE_PAGES.github, RELEASE_PAGES.gitee];
  return (
    <Dialog.Root open onOpenChange={(o) => !o && onClose()}>
      <Dialog.Portal>
        {/* z-[90] 与命令面板同档：Radix Portal 默认无 z-index，会被 sticky 表头（z-10）穿透
            （用户 2026-11-19 日志/Skills 页截图：表头横在弹层中间）。 */}
        <Dialog.Overlay className="fixed inset-0 z-[90] bg-black/60" />
        <Dialog.Content className="fixed left-1/2 top-1/2 z-[90] w-[440px] max-w-[92vw] -translate-x-1/2 -translate-y-1/2 rounded-xl border border-slate-700 bg-slate-900 p-5 shadow-2xl">
          <Dialog.Title className="text-base font-semibold text-ink-1">
            {t("ver.reposTitle")}
          </Dialog.Title>
          <Dialog.Description className="mt-0.5 text-sm text-ink-4">
            {t("ver.reposDesc")}
          </Dialog.Description>
          <ul className="mt-3 space-y-1.5">
            {links.map((url) => (
              <li key={url} className="flex items-center gap-1">
                <button
                  type="button"
                  // 应用内浏览器打开（口径 2026-11-19 由系统浏览器升级）：实现收在 lib/links.ts。
                  onClick={() => openExternal(url)}
                  className="flex min-w-0 flex-1 items-center gap-3 rounded border border-slate-700 bg-slate-800/40 px-3 py-2 text-left text-sm text-ink-2 hover:border-cyan-500/50 hover:text-ink-1"
                >
                  {/* 只留 URL（用户 2026-11-19 拍板删「GitHub/Gitee 发布页」标签）：
                      域名本身已说明是哪家，标签是重复信息。 */}
                  <span className="min-w-0 truncate font-mono text-2xs text-ink-4" title={url}>
                    {url}
                  </span>
                </button>
                {/* 一键复制（用户 2026-11-20 要求）：行钮是打开页面，复制得单独一个钮。 */}
                <CopyButton text={url} />
              </li>
            ))}
          </ul>
          <div className="mt-4 flex justify-end">
            <Button size="sm" onClick={onClose}>
              {t("upd.close")}
            </Button>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
