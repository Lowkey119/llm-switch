// 分配灯 / 槽位浮层的共用口径（后端页模型行与代理页矩阵同源同算法）：
// 占用表、格子占槽集合、槽位可读名、pi 槽位标签、代理列序。两处各写一遍迟早行为分叉。
import type { MatrixCell, MatrixClient, MatrixView } from "../../lib/ipc";
import { modelPrefix } from "../../lib/modelprefix";
import { t } from "../../lib/i18n";

/** 列序（UI-11，用户 2026-11-16）：Claude Code、Codex、Codebuddy、OpenCode、
 *  Pi、DevEco、Copilot、Gemini。后端 ClientApp::ALL 的顺序不动（别处共用），
 *  只在这里按用户口径重排；没登记的新代理排最后、保持原有相对顺序。 */
export const COL_ORDER = ["claude", "codex", "codebuddy", "opencode", "pi", "deveco", "copilot", "gemini"];

export function sortCols(cs: MatrixClient[]): MatrixClient[] {
  const rank = (app: string) => {
    const i = COL_ORDER.indexOf(app);
    return i === -1 ? COL_ORDER.length : i;
  };
  return [...cs].sort((a, b) => rank(a.app) - rank(b.app));
}

/** 槽位的可读名：pi 两个真实槽用角色词，档位制沿用档位键（main = 默认档）。 */
export function slotLabel(client: MatrixClient, slot: string): string {
  if (client.app === "pi") return slot === "small" ? t("ap.piSmall") : t("ap.piMain");
  return slot === "main" ? t("ap.defaultTier") : slot;
}

/** 一个格子占的槽位集合（默认档折算成 "main"）；无格子 = 空集。 */
export function cellSlots(cell?: MatrixCell): string[] {
  if (!cell) return [];
  const s = cell.slots?.length ? cell.slots : cell.slot ? [cell.slot] : [];
  return cell.is_default ? [...s, "main"] : s;
}

/** 占用表（app → slot → 模型）：浮层里标「将替换」的数据源。 */
export function buildOccupancy(view: MatrixView): Record<string, Record<string, string>> {
  const occ: Record<string, Record<string, string>> = {};
  for (const r of view.rows) {
    for (const c of view.clients) {
      const cell = r.cells[c.app];
      if (!cell) continue;
      const slots = cell.slots ?? (cell.slot ? [cell.slot] : []);
      for (const s of slots) (occ[c.app] ??= {})[s] = r.model;
      if (cell.is_default) (occ[c.app] ??= {})["main"] = r.model;
    }
  }
  return occ;
}

/** pi 槽位显示名 = 实配模型的有意义前缀（占着谁显示谁），空槽退回角色词；别家不定制。 */
export function piLabels(
  client: MatrixClient,
  slots: string[],
  occ: Record<string, string>,
): Record<string, string> | undefined {
  if (client.app !== "pi") return undefined;
  return Object.fromEntries(
    slots.map((s) => [s, occ[s] ? modelPrefix(occ[s]) : slotLabel(client, s)]),
  );
}
