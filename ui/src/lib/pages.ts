/**
 * 页面清单（计划 4.1 / 决策 D3）：PAGES 与边条 NAV 等长，没有进不去的孤儿页。
 * 旧的 models / cache 两页已并入后端页与设置页，PageKey 里不再存在。
 */
export type PageKey =
  | "backends"
  | "monitor"
  | "logs"
  | "usage"
  | "console"
  | "agents"
  | "skills"
  | "settings";

export const PAGES: Record<PageKey, { title: string }> = {
  backends: { title: "后端" },
  monitor: { title: "实时监控" },
  logs: { title: "日志" },
  usage: { title: "用量" },
  console: { title: "后端控制台" },
  agents: { title: "代理" },
  skills: { title: "Skills 管理" },
  settings: { title: "设置" },
};
