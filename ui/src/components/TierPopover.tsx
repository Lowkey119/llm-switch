// 槽位浮层（claude 六档 / codebuddy 三档 / pi 双槽共用）：选槽即占（顶替），被占槽标「将替换」；
// 已分配可取消。一个模型现在能占多个槽，current 因此是集合而不是单值。
//
// 拆分语义（后端页与矩阵页共用）：
// · onPick(slot)  —— 选一个槽位占住（slot="main" = 默认档 / pi 主模型）；
// · onRemove      —— 点「取消分配」（把当前模型从它的槽位上拔下来）；
// · labels        —— 槽位显示名映射（pi 传实配模型的有意义前缀，缺省显示槽位键）。
//
// 这里只渲染内容：定位、Esc、外点关闭、焦点进出都交给 PointPopover（components/Popover.tsx），
// 此前自己挂 mousedown 的写法既关不掉也进不去（UI-AUDIT P0-3）。
import { popoverItemCls } from "./Popover";
import { t, ts } from "../lib/i18n";

export function TierPopover({
  slots,
  labels,
  current,
  occ,
  self,
  onPick,
  onRemove,
}: {
  slots: string[];
  /** 槽位显示名（缺省 = 槽位键原样）。 */
  labels?: Record<string, string>;
  current?: string[];
  occ: Record<string, string>;
  self: string;
  onPick: (slot: string) => void;
  onRemove: () => void;
}) {
  return (
    <div className="min-w-[170px]">
      {slots.map((slot) => {
        const cur = !!current?.includes(slot);
        const holder = occ[slot];
        const own = holder === self;
        return (
          <button
            key={slot}
            onClick={() => onPick(slot)}
            className={popoverItemCls}
          >
            <span className={cur ? "text-[var(--sem-ok-text)]" : ""}>
              {labels?.[slot] ?? slot}
              {cur ? t("ap.current") : ""}
            </span>
            {holder && !own && <span className="text-sm text-ink-4">{ts("ap.willReplace", { holder })}</span>}
          </button>
        );
      })}
      {current && current.length > 0 && (
        <button
          onClick={onRemove}
          className="mt-1 block w-full border-t border-slate-800 py-1.5 text-center text-sm text-[var(--sem-danger-text)]"
        >
          {t("ap.unassign")}
        </button>
      )}
    </div>
  );
}
