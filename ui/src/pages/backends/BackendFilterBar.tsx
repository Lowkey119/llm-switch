import { t, ts } from "../../lib/i18n";
import { FilterBar } from "../../components/FilterBar";
import { SearchInput } from "../../components/SearchInput";
import { Tabs } from "../../components/Tabs";
import type { useBackendFilter } from "./useBackendFilter";
import type { BackendStatus } from "./useBackendFilter";

/**
 * 后端页过滤条：搜索 + 状态分段，只这两件。
 *
 * 用户 2026-11-19 截图点名 U6 加的三件全是多余的，撤：
 *   引擎下拉 —— 搜索框的关键词已含引擎名（`useBackendFilter` 的 hay 里有 engine），
 *               同一个维度摆两个入口，只会让人怀疑哪个在生效；
 *   命中计数 —— 这页就四五行，列表自己数得清，一个不参与决策的数字是噪音；
 *   已选 chip + 「清除过滤」—— 控件就在同一行、各自带复位（分段有「全部」、
 *               搜索框自带清空键），再复述一遍状态是把同一句话说两次。
 *
 * 状态与判据都在 `useBackendFilter` 里，这里只摆控件。
 */
export function BackendFilterBar({ bf }: { bf: ReturnType<typeof useBackendFilter> }) {
  return (
    <FilterBar>
      <SearchInput
        value={bf.q}
        onChange={bf.setQ}
        placeholder={t("bk.searchPh")}
        ariaLabel={ts("bk.searchAria")}
        inputClassName="w-52"
      />
      <Tabs
        variant="segmented"
        size="sm"
        ariaLabel={t("filter.byStatus")}
        value={bf.status}
        onChange={(id) => bf.setStatus(id as BackendStatus)}
        items={(
          [
            ["all", "filter.all"],
            ["enabled", "filter.enabled"],
            ["disabled", "filter.disabled"],
            ["abnormal", "filter.abnormal"],
          ] as const
        ).map(([id, key]) => ({ id, label: t(key) }))}
      />
    </FilterBar>
  );
}
