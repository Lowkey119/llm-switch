// 全站「网关已暴露模型按后端分组」的唯一取数口（fix:model-lists-group-by-backend）。
//
// 口径与接管页下拉同源（后端 `takeover::option_groups`）：独占模型归自家后端组，
// 多后端共享的模型归 `a + b` 组合组，每个模型恰好出现一次；名字是**暴露名**
// （用户没改过名时它与上游 id 同名，本来就是同一个字符串）。
// 凡是要列「网关暴露了哪些模型」的选择器/清单都从这里取，不许各自拿后端 rows 反推。
import { useQuery } from "@tanstack/react-query";
import { ipc } from "./ipc";
import { every, POLL } from "./poll";
import { t } from "./i18n";

export interface ModelGroup {
  backend: string;
  models: string[];
}

export const MODEL_GROUPS_KEY = ["model-groups"];

/** 后端没归组的模型（新加后端还没扫、或清单来自别处）落这一组，绝不因为分组就丢模型。 */
export function otherGroupLabel(): string {
  return t("mg.other");
}

/** 组名：多后端共享的模型归 `a + b`（与后端 option_groups 的拼法逐字一致）。 */
export const groupKey = (backends: string[]) => backends.join(" + ");

export function useModelGroups() {
  return useQuery<ModelGroup[]>({
    queryKey: MODEL_GROUPS_KEY,
    queryFn: () => ipc<ModelGroup[]>("model_groups"),
    // 跟着暴露清单走（矩阵那一档），比清单类再慢一档就够。
    refetchInterval: () => every(POLL.slow),
  });
}

/**
 * 把一批模型按后端分组排好：组序与组内序都跟着 `groups`（后端真值），
 * `groups` 里没有的模型归到末尾的「其它」组。
 *
 * 只展示其中一部分的地方（缓存页去掉已添加的、门面只列可见的）用它筛，
 * 这样各处看到的组顺序与组内顺序仍是同一套，不会每页各排各的。
 */
export function groupModels(groups: ModelGroup[], models: string[]): ModelGroup[] {
  const want = new Set(models);
  const taken = new Set<string>();
  const out: ModelGroup[] = [];
  for (const g of groups) {
    const ms = g.models.filter((m) => want.has(m) && !taken.has(m));
    ms.forEach((m) => taken.add(m));
    if (ms.length) out.push({ backend: g.backend, models: ms });
  }
  const rest = models.filter((m) => !taken.has(m));
  if (rest.length) out.push({ backend: otherGroupLabel(), models: rest });
  return out;
}
