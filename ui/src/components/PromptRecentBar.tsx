// 「最近发送」批量删除条：OK 勾选后在这里删除所选。
import { t } from "../lib/i18n";
import { Button } from "./ui";

export function PromptRecentBar({
  count,
  onDelete,
  onClear,
}: {
  count: number;
  onDelete: () => void;
  onClear: () => void;
}) {
  if (!count) return null;
  return (
    <div className="flex items-center gap-1.5 px-3 py-1 text-sm">
      <Button size="xs" variant="danger" onClick={onDelete}>
        {t("prompt.deletePicked").replace("{n}", String(count))}
      </Button>
      <Button size="xs" onClick={onClear}>
        {t("prompt.clearPicked")}
      </Button>
    </div>
  );
}
