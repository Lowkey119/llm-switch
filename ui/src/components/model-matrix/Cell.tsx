// 矩阵的满槽判定（ui_04 转置后格子直接列在格内，渲染逻辑回到 ModelMatrix 一处）。
// 口径仍单独住在这里：行头提示与浮层要用同一套，不能两处各写一遍。
import type { MatrixClient } from "../../lib/ipc";

/** 纯档位槽端（无清单兜底）：claude / codebuddy——它们只有档位槽，槽满就无处可分配；
 *  pi / codex 是「槽 + 清单」双轨，其余端是清单制，满了仍能进清单。 */
export const isTierOnly = (c: MatrixClient) => c.slots.length > 0 && c.app !== "pi" && c.app !== "codex";
/** 该端模型位是否已满（无 capacity = 清单制数量不限，永不「满」）。 */
export const isFull = (c: MatrixClient) => c.capacity != null && c.used >= c.capacity;
