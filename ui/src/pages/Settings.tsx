// 设置页（M3 范围）：路由策略与熔断的热更面板；浮窗/资源小窗/日志的开关面板。
// 版式约定：外壳 App.tsx 的 <section> 只留左右与底部 padding（sticky 穿透修复，
// 2026-11-20），顶部留白由各页根节点 pt-[var(--content-pad-top)] 自带（U9 起随窗口连续变化）。
// 页面自己不再补内边距（此前各页又叠了一层 p-5，全站 inset 变成 40px，只有后端/控制台是 20px）。
import { useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { QueryError } from "../components/QueryError";
import { useToast } from "../components/Toast";
import { Card, CardHead, FieldLabel, selectCls } from "../components/ui";

import { t, ts } from "../lib/i18n";
import { NumCell } from "../components/NumCell";
import { EndpointsCard } from "../components/EndpointsCard";
import { SysEngineCard } from "./monitor/SysMonCards";
import { GatewayOps } from "./settings/GatewayOps";
import { SaveBar } from "./settings/SaveBar";
import { PresetPanel } from "./settings/PresetPanel";
// 提示词速取面板的设置卡已迁至代理页「提示词与命令」（pages/agents/PromptsView）。
import { ToggleCards } from "./settings/ToggleCards";
import { ipc } from "../lib/ipc";

interface CircuitCfg {
  enabled: boolean;
  failure_threshold: number;
  success_threshold: number;
  timeout_secs: number;
  error_rate_threshold: number;
  min_requests: number;
}

interface RoutingCfg {
  policy: string;
  retries: number;
  cooldown_secs: number;
  circuit_breaker: CircuitCfg;
}

// 策略与熔断数值项的文案存 i18n key（模块级常量加载时定不了语言，渲染时取词）。
const POLICIES: [string, string][] = [
  ["weighted_round_robin", "set.policy.wrr"],
  ["round_robin", "set.policy.rr"],
  ["least_busy", "set.policy.lb"],
  ["failover", "set.policy.fo"],
];

// 熔断器的数值项：一次声明，渲染与脏检查都走这张表，不再手摆占位 div。
// 第 4 项是钳制上限（fix:ui-trust-and-form-exit）：错误率是 0~1 的比率，越界无意义。
const CB_NUMS: [keyof CircuitCfg, string, string, number?][] = [
  ["failure_threshold", "set.cb.failT", "set.cb.failTHint"],
  ["success_threshold", "set.cb.succT", "set.cb.succTHint"],
  ["timeout_secs", "set.cb.halfDelay", "set.cb.halfDelayHint"],
  ["error_rate_threshold", "set.cb.errRate", "set.cb.errRateHint", 1],
  ["min_requests", "set.cb.minReqs", "set.cb.minReqsHint"],
];

/**
 * 与基线逐键比对，返回改动项数（U7 保存条口径）。
 *
 * 原先只有一句 JSON.stringify 相等与否的布尔值，保存条上只能写「有未保存的修改」——
 * 改了三项和改了一项长得一样，用户无从判断自己到底动了什么。熔断器按子键计，
 * 和表单上「一项一格」的视觉粒度对齐。
 */
function countDirty(form: RoutingCfg, base: RoutingCfg) {
  let n = 0;
  (Object.keys(form) as (keyof RoutingCfg)[]).forEach((k) => {
    if (k === "circuit_breaker") return;
    if (form[k] !== base[k]) n++;
  });
  (Object.keys(form.circuit_breaker) as (keyof CircuitCfg)[]).forEach((k) => {
    if (form.circuit_breaker[k] !== base.circuit_breaker[k]) n++;
  });
  return n;
}

export function SettingsPage() {
  const qc = useQueryClient();
  const routing = useQuery({
    queryKey: ["routing"],
    queryFn: () => ipc<RoutingCfg>("get_routing"),
  });
  const [form, setForm] = useState<RoutingCfg | null>(null);
  // 非法数值字段登记（fix:ui-trust-and-form-exit）：任一无效即禁保存，就地标红说明。
  const [badNums, setBadNums] = useState<Record<string, boolean>>({});
  const setBad = (k: string) => (bad: boolean) =>
    setBadNums((s) => (s[k] === bad ? s : { ...s, [k]: bad }));
  const badCount = Object.values(badNums).filter(Boolean).length;
  // 放弃修改时基线复位但非法字段 value 从未变过（非法不回写），NumCell 靠 value
  // 变化收不回脏文本——用 tick 强制重挂，显示一律回到基线。
  const [resetTick, setResetTick] = useState(0);
  const toast = useToast();

  useEffect(() => {
    if (routing.data && !form) setForm(routing.data);
  }, [routing.data, form]);

  // 脏检查：form 是 routing.data 的浅拷贝 + 补丁，逐键比出「改了几项」，不只判有没有。
  const dirtyCount = form && routing.data ? countDirty(form, routing.data) : 0;

  const save = useMutation({
    mutationFn: () => ipc("set_routing", { routing: form }),
    onSuccess: () => {
      toast.ok(t("set.savedHot"));
      qc.invalidateQueries({ queryKey: ["routing"] });
    },
    onError: (e) => toast.err(ts("set.saveFail", { e: String(e) })),
  });

  // 网关日志级别 / 日志保留 / 预设一键还原：三张卡已整块搬去
  // pages/settings/GatewayOps.tsx（与路由/熔断表单无共享状态，且本页已长过门禁线）。

  // 读取失败必须显示失败，不能停在「载入中…」——不然整页像正常但永远改不了。
  // 复用页面错误边界的红卡样式（components/ErrorBoundary.tsx），给重试入口。
  if (routing.isError) {
    return (
      <div className="w-full space-y-4 pt-[var(--content-pad-top)]">
        <QueryError title={t("set.routingLoadFail")} error={routing.error} onRetry={() => routing.refetch()} />
      </div>
    );
  }

  if (!form) {
    return (
      <div className="w-full space-y-4 pt-[var(--content-pad-top)]">
        <div className="text-sm text-ink-4">{t("common.loading")}</div>
      </div>
    );
  }
  const set = (patch: Partial<RoutingCfg>) => setForm({ ...form, ...patch });
  const setCb = (patch: Partial<CircuitCfg>) =>
    setForm({ ...form, circuit_breaker: { ...form.circuit_breaker, ...patch } });

  return (
    <div className="w-full space-y-4 pt-[var(--content-pad-top)]">
      {/* 系统与引擎（用户点名置顶）：两台机器硬件指标 + 实时更新开关，与监控页同源。 */}
      <SysEngineCard />
      <EndpointsCard />
      {/* 三张开关卡（接单 / 多行浮动面板 / 资源悬浮小窗）整块在 settings/ToggleCards。 */}
      <ToggleCards />
      {/* 提示词速取面板的设置整体迁到代理页「提示词与命令」（用户 2026-11-18）：
          条目、快捷键、注入方式、辅助功能授权同一处管，设置页不再留第二个入口。 */}

      {/* 表单区自成一块：保存条 sticky 的边界就是这个容器——表单在屏幕上时它吸底，
          滚到下方「网关运维」就跟着表单一起走掉，不会拿表单的脏状态去压别的区。 */}
      <div className="space-y-4">
        <Card>
          <CardHead title={t("set.routingTitle")} />
          <div className="form-grid gap-3 text-sm">
            <FieldLabel label={t("set.policyLabel")}>
              <select
                value={form.policy}
                onChange={(e) => set({ policy: e.target.value })}
                className={selectCls}
              >
                {POLICIES.map(([v, labelKey]) => (
                  <option key={v} value={v}>
                    {t(labelKey)}
                  </option>
                ))}
              </select>
            </FieldLabel>
            <NumCell
              key={`retries${resetTick}`}
              label={t("set.retries")}
              value={form.retries}
              min={0}
              onCommit={(n) => set({ retries: n })}
              onValidity={setBad("retries")}
            />
            <NumCell
              key={`cooldown${resetTick}`}
              label={t("set.cooldown")}
              value={form.cooldown_secs}
              min={0}
              onCommit={(n) => set({ cooldown_secs: n })}
              onValidity={setBad("cooldown")}
            />
          </div>
        </Card>

        <Card>
          <CardHead title={t("set.cbTitle")}>
            <label className="flex shrink-0 cursor-pointer items-center gap-2 text-sm text-ink-2">
              <input
                type="checkbox"
                checked={form.circuit_breaker.enabled}
                onChange={(e) => setCb({ enabled: e.target.checked })}
              />
              {t("set.cbEnable")}
            </label>
          </CardHead>
          <div className={`form-grid gap-3 text-sm ${form.circuit_breaker.enabled ? "" : "dim-idle"}`}>
            {CB_NUMS.map(([key, labelKey, hintKey, max]) => (
              <NumCell
                key={`${key}${resetTick}`}
                label={t(labelKey)}
                hint={t(hintKey)}
                value={form.circuit_breaker[key] as number}
                min={0}
                max={max}
                step={key === "error_rate_threshold" ? "0.05" : "1"}
                disabled={!form.circuit_breaker.enabled}
                onCommit={(n) => setCb({ [key]: n } as Partial<CircuitCfg>)}
                onValidity={setBad(`cb:${key}`)}
              />
            ))}
          </div>
        </Card>

        <SaveBar
          dirtyCount={dirtyCount}
          badCount={badCount}
          saving={save.isPending}
          onSave={() => save.mutate()}
          onDiscard={() => {
            if (!routing.data) return;
            setForm(routing.data);
            // 非法值不回写表单，所以「放弃」时 value 可能一个字没变，NumCell 靠 value
            // 变化收不回脏文本——用 tick 强制重挂，显示一律回到基线。
            setResetTick((v) => v + 1);
          }}
        />
      </div>
      {/* 接管面板已搬入「代理」tab（docs/plans/2026-10-09-代理管理tab.md 片一）。 */}

      {/* 响应缓存卡整块撤下（用户 2026-11-17 拍板，见 docs/fix/archive/cache-always-bypass.md）：
          默认口径（max_temperature=0.0 + 缺省 temperature 按 1.0 + 带 tools 就排除）
          把编码客户端的流量全判成绕行，卡上永远是 0 命中 / N 绕行，而那两个旋钮 UI 里根本没有入口。
          真正省 token 的是推理引擎的 prefix/KV cache 与 provider 的 prompt cache
          （用量里的 cache_read_tokens 就是上游真值，照旧展示）。
          ⚠️ 只删前端：后端 `gateway/cache.rs`、`commands/cache.rs` 与那对 IPC 全部保留，
          要恢复 UI 把 CacheCard 取回来即可。 */}

      {/* 网关运维（日志级别 / 日志保留 / 预设与归档）+ 关于：
          整块收在 pages/settings/ 下，本页只留路由与熔断表单。
          PresetPanel 是一张合并卡（fix:merge-preset-cards，用户 2026-11-17）：
          原先「按代理逐项还原 agent-configs」与 GatewayOps 里的「整包归档还原」
          分成两张卡、两个都叫「还原」却还原回不同东西，现已并成一张，来源写在按钮上。 */}
      <section className="space-y-4 border-t border-slate-800 pt-5">
        <h2 className="text-base font-semibold">{t("set.opsTitle")}</h2>
        <PresetPanel />
        <GatewayOps />
      </section>

    </div>
  );
}
