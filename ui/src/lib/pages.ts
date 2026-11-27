/**
 * 页面清单（融合原型 unified.html 回灌：9 页并 5 页，按功能分类）。
 * 观测 = 实时监控 + 用量 + 日志；后端 = 配置 + 控制台；代理 = 代理管理 + Skills。
 * 被融合的原页面组件本体零改动，由壳页（Observe / Backends / Agents）挂页签。
 * 旧的 models / cache / monitor / logs / usage / console / skills 独立页键均已并入，PageKey 里不再存在。
 */
export type PageKey = "observe" | "backends" | "agents" | "remote" | "settings";

/** 标题存 i18n key（page.*），消费方 t() 取词。 */
export const PAGES: Record<PageKey, { titleKey: string }> = {
  observe: { titleKey: "page.observe" },
  backends: { titleKey: "page.backends" },
  agents: { titleKey: "page.agents" },
  remote: { titleKey: "page.remote" },
  settings: { titleKey: "page.settings" },
};
