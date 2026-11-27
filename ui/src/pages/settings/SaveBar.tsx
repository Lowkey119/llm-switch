import { Button } from "../../components/ui";
import { IconCheck, IconRefresh } from "../../components/icons";
import { t, ts } from "../../lib/i18n";

/**
 * 设置页吸底保存条（ui_01 原型第 6 条回灌，U7）。
 *
 * 原先保存 / 放弃两颗按钮排在表单末尾的普通流里：往下滚去核熔断参数，保存条就滚没了；
 * 改完再滚回来还得重新找那颗钮。状态也只有一句「有未保存的修改」——不说改了几项，
 * 也不说为什么按不动。这里改成吸底：表单还在屏幕上期间它一直贴着滚动区底边，
 * 滚过表单区（进下方「网关运维」）就跟着表单一起走掉，不拿自己的状态去压别的区。
 *
 * 三态按计划口径：非法 / 未保存 / 保存中，另给一个「已全部保存」的干净态收尾
 * （干净时按钮禁用但入口还在，比整条忽隐忽现少一次跳版）。
 * 计数只报与基线逐键比出来的真实改动项数，不写「有修改」这种含糊话。
 *
 * 「放弃修改」在**非法但没改脏**时也要能按：非法值根本不回写表单，于是这种状态
 * 完全可达（把错误率填成 5 就是），此时它是唯一的退路。
 */
export function SaveBar({
  dirtyCount,
  badCount,
  saving,
  onSave,
  onDiscard,
}: {
  /** 与基线不一致的项数（熔断器按子键计）。 */
  dirtyCount: number;
  /** 取值非法的项数。非零即禁保存。 */
  badCount: number;
  saving: boolean;
  onSave: () => void;
  onDiscard: () => void;
}) {
  const blocked = badCount > 0;
  const dirty = dirtyCount > 0;
  // 状态文字自带语义，颜色只是加强（UI 法条：状态不得只靠颜色表达）。
  const tone = blocked
    ? "text-[var(--sem-danger-text)]"
    : dirty
      ? "text-[var(--sem-busy-text)]"
      : "text-ink-4";
  return (
    <div className="sticky bottom-0 z-20 flex flex-wrap items-center gap-2 rounded-lg border border-slate-700 bg-slate-900 px-4 py-3 text-sm shadow-2xl">
      <span aria-live="polite" className={tone}>
        {blocked ? ts("save.bad", { n: badCount }) : dirty ? ts("save.dirty", { n: dirtyCount }) : t("save.clean")}
      </span>
      <span className="ml-auto flex items-center gap-2">
        <Button onClick={onDiscard} disabled={saving || (!dirty && !blocked)}>
          {t("save.discard")}
        </Button>
        <Button variant="primary" onClick={onSave} disabled={saving || blocked || !dirty}>
          <span className="flex items-center gap-1.5">
            {saving ? (
              <IconRefresh width={12} height={12} className="animate-spin" aria-hidden="true" />
            ) : (
              <IconCheck width={12} height={12} aria-hidden="true" />
            )}
            {saving ? t("save.saving") : t("save.save")}
          </span>
        </Button>
      </span>
    </div>
  );
}
