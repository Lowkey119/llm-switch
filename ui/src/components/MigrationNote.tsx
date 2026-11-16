import { useState } from "react";
import { IconClose } from "./icons";

/**
 * 一次性迁移指引（计划 4.3）。
 *
 * 信息架构动过之后，老用户的肌肉记忆会去找「模型与参数」「缓存」那两页。
 * 指引只在被关掉一次之后才永久消失（存 localStorage），而不是按时间自动过期 ——
 * 自动过期意味着每个人至少会撞一次「页面不见了」才知道搬家了。
 */
export function MigrationNote({ id, children }: { id: string; children: React.ReactNode }) {
  const key = `llm-switch.migrated.${id}`;
  const [hidden, setHidden] = useState(() => {
    try {
      return localStorage.getItem(key) === "1";
    } catch {
      return false;
    }
  });
  if (hidden) return null;
  const dismiss = () => {
    try {
      localStorage.setItem(key, "1");
    } catch {
      /* 无痕模式写不进去，本次会话内仍然隐藏 */
    }
    setHidden(true);
  };
  return (
    <div className="flex items-start gap-2 rounded-lg border border-slate-700 bg-slate-900/60 px-3 py-2 text-xs leading-relaxed text-ink-3">
      <span className="min-w-0 flex-1">{children}</span>
      <button
        onClick={dismiss}
        aria-label="知道了，不再提示"
        className="shrink-0 rounded p-0.5 text-ink-4 transition-colors hover:text-ink-2"
      >
        <IconClose width={12} height={12} />
      </button>
    </div>
  );
}
