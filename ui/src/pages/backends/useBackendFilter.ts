import { useState } from "react";
import type { BackendRow } from "../../lib/ipc";

/**
 * 后端页过滤（ui_01 回灌 U6；引擎维度按用户 2026-11-19 口径撤掉）。
 *
 * 只留两个维度，都取真实字段，不造没有数据支撑的筛选项：
 *   全文搜 —— 名称 / base_url / 引擎 / 协议 / 模型名（引擎靠这里搜，不再单开下拉）
 *   状态   —— 启用、停用，以及「异常」（就是状态灯那两档 down / degraded，不另造判据）
 *
 * 单列成文件是因为 Backends.tsx 已经过了计划 §6.1 的 400 行目标线，
 * 过滤这块又是一整个能独立读懂的单元，塞在页面里只会把它推得更远。
 */

export type BackendStatus = "all" | "enabled" | "disabled" | "abnormal";

export function useBackendFilter(all: BackendRow[]) {
  const [q, setQ] = useState("");
  // 默认「启用」（用户 2026-11-19 拍板）：日常关心的是在跑的后端，停用的本来就少，要看再切。
  const [status, setStatus] = useState<BackendStatus>("enabled");

  const filtered = !!q.trim() || status !== "all";
  const rows = all.filter((b) => {
    if (status === "enabled" && !b.enabled) return false;
    if (status === "disabled" && b.enabled) return false;
    if (status === "abnormal" && b.health_state !== "down" && b.health_state !== "degraded") return false;
    const kw = q.trim().toLowerCase();
    if (kw) {
      const hay = `${b.name} ${b.base_url} ${b.engine ?? ""} ${b.protocol} ${b.models.join(" ")}`.toLowerCase();
      if (!hay.includes(kw)) return false;
    }
    return true;
  });
  /** 一键复位：只清这页真有的两个维度（空态的「清除过滤」入口用）。 */
  const clear = () => {
    setQ("");
    setStatus("all");
  };

  return {
    rows,
    filtered,
    clear,
    q,
    setQ,
    status,
    setStatus,
  };
}
