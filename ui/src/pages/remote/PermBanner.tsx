// 遥控器页顶部的系统授权告警（用户 2026-11-27 拍板：告警放在遥控器映射页，
// 不放快速面板——面板正是靠遥控器按键唤起的，按键被授权卡住时面板里没人看得到）。
// 复用后端 remote_perms_check / remote_perm_open 和 i18n 现成词条，不新造 IPC。
// 非 macOS 没有遥控器功能，那两个命令不存在、查询会失败：整块不渲染，别用报错卡噪声。
import { useQuery } from "@tanstack/react-query";
import { t } from "../../lib/i18n";
import { ipc } from "../../lib/ipc";

interface Perm {
  id: string;
  granted: boolean;
}

/** 只认认识的 id；后端将来加项而词典没跟上时，不显示裸 key。 */
const KNOWN = new Set(["a11y", "input_monitor", "speech", "mic", "bluetooth"]);

export default function PermBanner() {
  const q = useQuery({
    queryKey: ["remote-perms"],
    queryFn: () => ipc<Perm[]>("remote_perms_check"),
    retry: false,
    staleTime: 30_000,
  });
  const missing = (q.data ?? []).filter((p) => !p.granted && KNOWN.has(p.id));
  if (q.isError || missing.length === 0) return null;
  return (
    <div className="mb-2 rounded-md border border-[var(--sem-danger-text)]/30 bg-[var(--sem-danger-text)]/10 px-3 py-2">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
        <span className="text-sm font-semibold text-[var(--sem-danger-text)]">
          {t("remote.perm.title")}
        </span>
        {missing.map((p) => (
          <span key={p.id} className="flex items-center gap-1.5 text-sm text-[var(--sem-danger-text)]">
            <span className="font-medium">{t(`remote.perm.${p.id}`)}</span>
            <span className="text-ink-3">{t(`remote.perm.${p.id}Desc`)}</span>
            <button
              type="button"
              className="rounded px-1.5 py-0.5 ring-1 ring-inset ring-[var(--sem-danger-text)]/50 hover:bg-[var(--sem-danger-text)]/15"
              onPointerDown={(e) => e.stopPropagation()}
              onClick={() => void ipc("remote_perm_open", { id: p.id })}
            >
              {t("remote.perm.open")}
            </button>
          </span>
        ))}
        <button
          type="button"
          className="rounded px-1.5 py-0.5 text-ink-2 ring-1 ring-inset ring-slate-700 hover:bg-slate-700/50"
          onPointerDown={(e) => e.stopPropagation()}
          onClick={() => void q.refetch()}
        >
          {t("remote.perm.refresh")}
        </button>
      </div>
    </div>
  );
}
