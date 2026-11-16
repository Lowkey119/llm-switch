// 设置页（M3 范围）：路由策略与熔断的热更面板；浮窗/资源小窗/日志的开关面板。
// 版式约定：外壳 App.tsx 的 <section> 已经带 p-5，页面自己不再补内边距
// （此前各页又叠了一层 p-5，全站 inset 变成 40px，只有后端/控制台是 20px）。
import { useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { QueryError } from "../components/QueryError";
import { useToast } from "../components/Toast";
import { Button, Card, CardHead, FieldLabel, selectCls } from "../components/ui";
import { NumCell } from "../components/NumCell";
import { CacheCard } from "../components/CacheCard";
import { EndpointsCard } from "../components/EndpointsCard";
import { SysEngineCard } from "./monitor/SysMonCards";
import { GatewayOps } from "./settings/GatewayOps";
import { PresetPanel } from "./settings/PresetPanel";
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

const POLICIES: [string, string][] = [
  ["weighted_round_robin", "加权轮转（默认）"],
  ["round_robin", "轮询"],
  ["least_busy", "最少负载（活跃流 + 近期 TTFT）"],
  ["failover", "故障转移（按配置顺序）"],
];

// 熔断器的数值项：一次声明，渲染与脏检查都走这张表，不再手摆占位 div。
// 第 4 项是钳制上限（fix:ui-trust-and-form-exit）：错误率是 0~1 的比率，越界无意义。
const CB_NUMS: [keyof CircuitCfg, string, string, number?][] = [
  ["failure_threshold", "连续失败开断阈值", "连续失败多少次后开断"],
  ["success_threshold", "半开恢复所需成功数", "开断后连续成功多少次合上"],
  ["timeout_secs", "开断后半开延迟（秒）", "开断多久后放一条试探请求"],
  ["error_rate_threshold", "错误率开断阈值（0~1）", "滑动窗口内错误率超过该值即开断", 1],
  ["min_requests", "错误率生效的最小请求数", "样本太少时不看错误率，避免误开断"],
];

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
  const anyBad = Object.values(badNums).some(Boolean);
  // 放弃修改时基线复位但非法字段 value 从未变过（非法不回写），NumCell 靠 value
  // 变化收不回脏文本——用 tick 强制重挂，显示一律回到基线。
  const [resetTick, setResetTick] = useState(0);
  const toast = useToast();

  useEffect(() => {
    if (routing.data && !form) setForm(routing.data);
  }, [routing.data, form]);

  // 脏检查：form 是 routing.data 的浅拷贝 + 补丁，键序稳定，串比即可。
  const dirty = !!form && !!routing.data && JSON.stringify(form) !== JSON.stringify(routing.data);

  const save = useMutation({
    mutationFn: () => ipc("set_routing", { routing: form }),
    onSuccess: () => {
      toast.ok("已保存并热生效");
      qc.invalidateQueries({ queryKey: ["routing"] });
    },
    onError: (e) => toast.err(`保存失败：${e}`),
  });

  // 网关日志级别 / 日志保留 / 预设一键还原：三张卡已整块搬去
  // pages/settings/GatewayOps.tsx（与路由/熔断表单无共享状态，且本页已长过门禁线）。

  // 读取失败必须显示失败，不能停在「载入中…」——不然整页像正常但永远改不了。
  // 复用页面错误边界的红卡样式（components/ErrorBoundary.tsx），给重试入口。
  if (routing.isError) {
    return (
      <div className="mx-auto w-full max-w-content">
        <QueryError title="读取路由配置失败" error={routing.error} onRetry={() => routing.refetch()} />
      </div>
    );
  }

  if (!form) {
    return <div className="mx-auto w-full max-w-content text-sm text-ink-4">载入中…</div>;
  }
  const set = (patch: Partial<RoutingCfg>) => setForm({ ...form, ...patch });
  const setCb = (patch: Partial<CircuitCfg>) =>
    setForm({ ...form, circuit_breaker: { ...form.circuit_breaker, ...patch } });

  return (
    <div className="mx-auto w-full max-w-content space-y-4">
      {/* 系统与引擎（用户点名置顶）：两台机器硬件指标 + 实时更新开关，与监控页同源。 */}
      <SysEngineCard />
      <EndpointsCard />
      {/* 三张开关卡（接单 / 多行浮动面板 / 资源悬浮小窗）整块在 settings/ToggleCards。 */}
      <ToggleCards />

      <Card>
        <CardHead title="路由策略" />
        <div className="grid grid-cols-1 gap-3 text-xs sm:grid-cols-2">
          <FieldLabel label="策略">
            <select
              value={form.policy}
              onChange={(e) => set({ policy: e.target.value })}
              className={selectCls}
            >
              {POLICIES.map(([v, label]) => (
                <option key={v} value={v}>
                  {label}
                </option>
              ))}
            </select>
          </FieldLabel>
          <NumCell
            key={`retries${resetTick}`}
            label="失败后额外重试次数"
            value={form.retries}
            min={0}
            onCommit={(n) => set({ retries: n })}
            onValidity={setBad("retries")}
          />
          <NumCell
            key={`cooldown${resetTick}`}
            label="失败冷却（秒）"
            value={form.cooldown_secs}
            min={0}
            onCommit={(n) => set({ cooldown_secs: n })}
            onValidity={setBad("cooldown")}
          />
        </div>
      </Card>

      <Card>
        <CardHead title="熔断器">
          <label className="flex shrink-0 cursor-pointer items-center gap-2 text-xs text-ink-2">
            <input
              type="checkbox"
              checked={form.circuit_breaker.enabled}
              onChange={(e) => setCb({ enabled: e.target.checked })}
            />
            启用熔断
          </label>
        </CardHead>
        <div className={`grid grid-cols-1 gap-3 text-xs sm:grid-cols-2 ${form.circuit_breaker.enabled ? "" : "dim-idle"}`}>
          {CB_NUMS.map(([key, label, hint, max]) => (
            <NumCell
              key={`${key}${resetTick}`}
              label={label}
              hint={hint}
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

      <div className="flex items-center gap-3">
        <Button
          variant="primary"
          disabled={save.isPending || !dirty || anyBad}
          onClick={() => save.mutate()}
        >
          保存（热生效）
        </Button>
        {/* 放弃修改（fix:ui-trust-and-form-exit）：改坏了要有退路，一键回到上次载入的基线。 */}
        <Button
          disabled={save.isPending || (!dirty && !anyBad)}
          onClick={() => {
            if (!routing.data) return;
            setForm(routing.data);
            setResetTick((v) => v + 1);
          }}
        >
          放弃修改
        </Button>
        {anyBad && <span className="text-xs text-[var(--sem-danger-text)]">有非法输入，修正后才能保存</span>}
        {dirty && !anyBad && <span className="text-xs text-[var(--sem-busy-text)]">有未保存的修改</span>}
      </div>
      {/* 接管面板已搬入「代理」tab（docs/plans/2026-10-09-代理管理tab.md 片一）。 */}

      {/* 响应缓存（计划 4.1 / 决策 D3）：它本质是网关的一个开关加效果读数，归设置。 */}
      <section className="space-y-4 border-t border-slate-800 pt-5">
        <h2 className="text-sm font-semibold">响应缓存</h2>
        <CacheCard />
      </section>

      {/* 网关运维（日志级别 / 日志保留 / 预设一键还原）+ 关于：
          整块收在 pages/settings/ 下，本页只留路由与熔断表单。
          PresetPanel（按代理清单还原 agent-configs）自代理页「模型分配全景」迁入
          （用户 2026-11-16「把这个迁移到设置模块中」）——与上面 GatewayOps 的
          「预设配置归档」（整目录归档）是两回事：这里按代理逐项比对/还原。 */}
      <section className="space-y-4 border-t border-slate-800 pt-5">
        <h2 className="text-sm font-semibold">网关运维</h2>
        <PresetPanel />
        <GatewayOps />
      </section>

    </div>
  );
}
