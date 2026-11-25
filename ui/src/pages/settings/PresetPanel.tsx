// 预设与归档（合并卡，fix:merge-preset-cards）。
//
// 原先是两张卡各说各话：本组件「预设配置一键还原」（按代理逐项比对 agent-configs）
// 与 GatewayOps 里的「预设配置归档」（整包归档，用户备份优先、否则内置预设）。
// 两个按钮都叫「还原」，还原的却是不同的东西，用户点哪个都不知道会覆盖回什么——
// 连文件头都得写注释解释「这是两回事」。需要注释解释的两个按钮就是该合并的按钮。
//
// 合并后三个动作，来源全写在标签上，绝不出现两个只叫「还原」的按钮：
//   备份当前配置 / 整包还原（自己的备份｜内置预设）/ 按代理还原（agent-configs）
// IPC 一律复用现成的五个命令，不动后端。
import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useToast } from "../../components/Toast";
import { Button, Card } from "../../components/ui";
import { IconCaret } from "../../components/icons";
import { ConfirmDialog, Risk } from "../../components/ConfirmDialog";
import { QueryError } from "../../components/QueryError";
import { ipc } from "../../lib/ipc";
import { t, tf, ts } from "../../lib/i18n";

/** 按代理逐项比对的存档条目（`preset_status`）。 */
interface PresetEntry {
  app: string;
  target: string;
  source: string;
  source_exists: boolean;
  target_exists: boolean;
  differs: boolean;
}

/** 整包归档读数（`config_archive_info`）。 */
interface ArchiveInfo {
  bytes: number;
  entries: number;
  top: string[];
  source: "user" | "embedded" | "broken";
  updated_ms: number;
  path: string;
  history: number;
}

const SOURCE_LABEL: Record<ArchiveInfo["source"], string> = {
  user: "set.src.user",
  embedded: "set.src.embedded",
  broken: "set.src.broken",
};

/** 归档读数一行话：来源 · 大小 · 项数（· 更新时间 · 历史份数）。 */
function archiveDesc(info?: ArchiveInfo): string {
  if (!info) return t("set.archiveFail");
  const parts = [
    t(SOURCE_LABEL[info.source]),
    `${(info.bytes / 1024 / 1024).toFixed(1)}MB`,
    ts("set.entriesN", { n: info.entries }),
  ];
  if (info.source === "user" && info.updated_ms) {
    parts.push(ts("set.updatedAt", { t: new Date(info.updated_ms).toLocaleString() }));
  }
  if (info.source === "user" && info.history) parts.push(ts("set.historyN", { n: info.history }));
  return parts.join(" · ");
}

export function PresetPanel() {
  const qc = useQueryClient();
  const toast = useToast();
  const [open, setOpen] = useState(false);
  /** 三个闸门各管各的：还原来源不同，确认文案必须分别说清。 */
  const [confirm, setConfirm] = useState<{ app: string | null } | null>(null);
  const [confirmArchive, setConfirmArchive] = useState(false);

  // 折叠态也要报「有 N 项有差异」，所以两个读数都常开（都是本地目录扫描，不联网）。
  const preset = useQuery({
    queryKey: ["preset"],
    queryFn: () => ipc<{ dir: string; entries: PresetEntry[] }>("preset_status"),
  });
  const archiveInfo = useQuery({
    queryKey: ["config-archive-info"],
    queryFn: () => ipc<ArchiveInfo>("config_archive_info"),
  });

  /** 还原后要让接管/矩阵一起对上：配置被整批换掉了。 */
  const invalidateAfterRestore = () => {
    qc.invalidateQueries({ queryKey: ["preset"] });
    qc.invalidateQueries({ queryKey: ["takeover"] });
    qc.invalidateQueries({ queryKey: ["matrix"] });
  };

  const restore = useMutation({
    mutationFn: (app: string | null) => ipc("preset_restore", { app }),
    onSuccess: (res: unknown) => {
      const r = res as { restored: unknown[]; snapshot: string };
      toast.ok(
        ts("set.restoredN", { n: r.restored.length }) +
          (r.snapshot ? ts("set.restoredSnap", { s: r.snapshot }) : ""),
      );
      setConfirm(null);
      invalidateAfterRestore();
    },
    onError: (e) => toast.err(ts("set.restoreFail", { e: String(e) })),
  });

  const restoreArchive = useMutation({
    mutationFn: () =>
      ipc<{
        restored: number;
        backed_up: number;
        unchanged: number;
        skipped: { path: string; reason: string }[];
        backup_dir: string;
        source: string;
      }>("config_archive_restore"),
    onSuccess: (r) => {
      const skip = r.skipped.length ? ts("set.skippedN", { n: r.skipped.length }) : "";
      const from = r.source === "user" ? t("set.src.user") : t("set.src.embedded");
      toast.ok(
        ts("set.archiveRestored", { from, n: r.restored, b: r.backed_up, u: r.unchanged }) + skip,
      );
      setConfirmArchive(false);
      archiveInfo.refetch();
      invalidateAfterRestore();
    },
    onError: (e) => toast.err(ts("set.restoreFail", { e: String(e) })),
  });

  const backupArchive = useMutation({
    mutationFn: () =>
      ipc<{
        files: number;
        bytes: number;
        target: string;
        skipped: { path: string; reason: string }[];
        history: number;
      }>("config_archive_backup"),
    onSuccess: (r) => {
      const skip = r.skipped.length ? ts("set.skippedN", { n: r.skipped.length }) : "";
      toast.ok(
        ts("set.backupDone", {
          n: r.files,
          mb: (r.bytes / 1024 / 1024).toFixed(1),
          skip,
          h: r.history,
        }),
      );
      archiveInfo.refetch();
    },
    onError: (e) => toast.err(ts("set.backupFail", { e: String(e) })),
  });

  const entries = preset.data?.entries ?? [];
  const changed = entries.filter((e) => e.differs && e.source_exists).length;
  const info = archiveInfo.data;
  const busy = restore.isPending || restoreArchive.isPending || backupArchive.isPending;
  const pending = (m: { isPending: boolean; variables?: unknown }) => m.isPending;

  return (
    <Card className="space-y-2 py-3">
      {/* ── 折叠头：收起也要能看懂「归档是什么、有几项待还原」 ── */}
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-sm">
        <button
          type="button"
          onClick={() => setOpen(!open)}
          className="inline-flex items-center gap-1 font-medium text-ink-2 hover:text-ink-1"
        >
          <IconCaret open={open} />
          {t("set.presetTitle")}
        </button>
        <span className="text-ink-4">{t("set.archiveLabel")}{t("common.colon")}{archiveDesc(info)}</span>
        {preset.isLoading ? (
          <span className="text-ink-4">{t("set.readingArchive")}</span>
        ) : changed > 0 ? (
          <span className="text-[var(--sem-busy-text)]">{ts("set.diffN", { n: changed })}</span>
        ) : (
          <span className="text-ink-4">{t("set.noDiff")}</span>
        )}
      </div>

      {/* ── 动作区：来源写在按钮上，两个还原不许再撞名 ── */}
      <div className="flex flex-wrap items-center gap-1.5 text-sm">
        <Button
          size="xs"
          disabled={busy}
          title={t("set.backupTip")}
          onClick={() => backupArchive.mutate()}
        >
          {pending(backupArchive) ? t("set.backingUp") : t("set.backupBtn")}
        </Button>
        <Button
          size="xs"
          variant="danger"
          disabled={busy}
          onClick={() => setConfirmArchive(true)}
        >
          {pending(restoreArchive)
            ? t("set.archiveRestoring")
            : ts("set.archiveRestore", { src: info ? t(SOURCE_LABEL[info.source]) : t("set.unreadable") })}
        </Button>
        <Button
          size="xs"
          variant="danger"
          disabled={busy || changed === 0}
          title={ts("set.compareTip", { dir: preset.data?.dir ?? "~/.llm-switch/agent-configs" })}
          onClick={() => setConfirm({ app: null })}
        >
          {pending(restore)
            ? t("set.restoring")
            : t("set.restoreByAgent") +
              (changed > 0 ? ts("set.parenN", { n: changed }) : t("set.parenNoDiff"))}
        </Button>
        <span className="text-ink-4">{t("set.snapshotNote")}</span>
      </div>

      {backupArchive.data && (
        <p className="text-sm text-ink-3">
          {ts("set.backupResult", {
            n: backupArchive.data.files,
            mb: (backupArchive.data.bytes / 1024 / 1024).toFixed(1),
            target: backupArchive.data.target,
          })}
          {backupArchive.data.skipped.length
            ? ts("set.skippedN", { n: backupArchive.data.skipped.length })
            : ""}
        </p>
      )}
      {backupArchive.isError && (
        <p className="text-sm text-[var(--sem-danger-text)]">{ts("set.backupFail", { e: String(backupArchive.error) })}</p>
      )}
      {restoreArchive.data && (
        <p className="text-sm text-ink-3">
          {ts("set.archiveResult", {
            r: restoreArchive.data.restored,
            b: restoreArchive.data.backed_up,
            u: restoreArchive.data.unchanged,
          })}
          {restoreArchive.data.backup_dir
            ? ts("set.snapshotDir", { d: restoreArchive.data.backup_dir })
            : ""}
        </p>
      )}
      {restoreArchive.isError && (
        <p className="text-sm text-[var(--sem-danger-text)]">
          {ts("set.archiveRestoreFail", { e: String(restoreArchive.error) })}
        </p>
      )}
      {archiveInfo.isError && (
        <QueryError inline title={t("set.archiveReadFail")} error={archiveInfo.error} onRetry={() => archiveInfo.refetch()} />
      )}
      {preset.isError && (
        <QueryError inline title={t("set.presetReadFail")} error={preset.error} onRetry={() => preset.refetch()} />
      )}

      {/* ── 展开区：逐代理差异清单（单项还原仍在这里） ── */}
      {open && (
        <ul className="space-y-1">
          {entries.map((e) => (
            <li key={`${e.app}:${e.target}`} className="flex items-center gap-2 text-sm">
              <span
                className={`h-1.5 w-1.5 shrink-0 rounded-full ${
                  !e.source_exists
                    ? "bg-[var(--dot-idle)]"
                    : e.differs
                      ? "bg-amber-400"
                      : "bg-emerald-400"
                }`}
              />
              <span className="w-20 shrink-0 text-ink-2">{e.app}</span>
              <span className="truncate text-ink-4" title={e.target}>{e.target}</span>
              <span className="shrink-0 text-ink-4">
                {!e.source_exists ? t("set.missing") : e.differs ? t("set.differs") : t("set.same")}
              </span>
              {e.differs && e.source_exists && (
                <button
                  type="button"
                  onClick={() => setConfirm({ app: e.app })}
                  className="ml-auto shrink-0 rounded border border-slate-700 px-1.5 py-0.5 text-ink-3 hover:text-cyan-300"
                >
                  {t("set.restoreThis")}
                </button>
              )}
            </li>
          ))}
        </ul>
      )}

      {/* 闸门一：按代理还原（agent-configs 存档） */}
      <ConfirmDialog
        open={!!confirm}
        title={t("set.restoreConfirmTitle")}
        danger
        confirmLabel={t("set.restore")}
        onClose={() => setConfirm(null)}
        onConfirm={() => restore.mutate(confirm?.app ?? null)}
      >
        {tf("set.restoreConfirmBody", {
          dir: <b>{preset.data?.dir ?? "~/.llm-switch/agent-configs"}</b>,
          scope: confirm?.app ? ts("set.overrideOne", { app: confirm.app }) : t("set.overrideAll"),
          snap: <b>.restore-backups/</b>,
        })}
      </ConfirmDialog>

      {/* 闸门二：整包还原（用户归档优先，否则内置预设） */}
      {confirmArchive && (
        <ConfirmDialog
          open
          title={
            info?.source === "user" ? t("set.restoreUserTitle") : t("set.restoreEmbeddedTitle")
          }
          confirmLabel={t("set.confirmRestore")}
          danger
          onClose={() => setConfirmArchive(false)}
          onConfirm={() => restoreArchive.mutate()}
        >
          {tf("set.restoreArchiveBody", {
            path: <b>~/.llm-switch</b>,
            snap: <b>config-restore-backups/</b>,
            risk: <Risk>{t("set.riskGatewayRead")}</Risk>,
          })}
          {info?.source === "user" ? (
            <>
              <br />
              {tf("set.usedUser", { b: <b>{t("set.src.user")}</b>, desc: archiveDesc(info) })}
            </>
          ) : (
            <>
              <br />
              {tf("set.usedEmbedded", { b: <b>{t("set.embeddedBold")}</b> })}
            </>
          )}
        </ConfirmDialog>
      )}
    </Card>
  );
}
