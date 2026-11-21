// 代理 tab（docs/plans/2026-10-09-代理管理tab.md）：可接管客户端的统一管理。
//
// 片一：左侧按「多档 / 单槽」分组的代理列表 + 右侧选中代理详情（接管 / 配置文件编辑），
//       接管面板与配置编辑器自设置页原样搬入。
// 片二：子视图切换 —— 「模型分配全景」（自模型与参数页搬入）；详情内补「模型分配」节。
//       片二原含「技能分配全景」与详情「技能分配」节，2026-10-17 按队列③回搬 Skills 页
//       （方案 B，回滚 2026-10-09 分工）：分配唯一入口按域分家——模型归代理页，技能归 Skills 页。
// 同源规矩：列表与分组取自 matrix_get（与模型矩阵同一份客户端清单，D24）；
// 接管详情沿用 takeover_* IPC。
//
// 本文件只留「页面骨架」：子视图切换、选中谁、跨页跳转落点。
// 列表 / 详情 / 备份 / 路径编辑 / 预设面板各自成文件（计划 §6.1），行为与拆分前一致。
import { useCallback, useEffect, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Card, Button } from "../components/ui";

import { t } from "../lib/i18n";
import { ipc, type AgentScan, type ClientCol, type MatrixView } from "../lib/ipc";
import { Tabs } from "../components/Tabs";
import { QueryError } from "../components/QueryError";
import ModelMatrix from "../components/ModelMatrix";
import { ConfigPanel } from "../components/ConfigPanel";
import { AgentList } from "./agents/AgentList";
import { AgentDetail } from "./agents/AgentDetail";
import { FacadeDetail } from "./agents/FacadeDetail";
import { InstallPanel } from "./agents/InstallPanel";
import { PromptsView } from "./agents/PromptsView";
import { SkillsPage } from "./Skills";
import type { TakeoverStatus } from "./agents/types";

/**
 * 子视图：代理管理为主，模型分配全景自旧页搬入（片二）。
 * 各端槽位的占槽管理就在代理详情的「模型分配」里做（claude 角色表已删：
 * 同一份数据两个入口只会让人分不清该改哪边）。技能分配全景在 Skills 页（队列③）。
 */
const VIEWS = [
  ["agents", "ag.view.agents"],
  ["models", "ag.view.models"],
  ["install", "ag.view.install"],
  // 提示词速取面板的内容与行为统一在这管（用户 2026-11-18）：条目本来就是
  // 「各家代理的常用命令」，与代理同页；设置页那张卡已迁走，不留双入口。
  ["prompts", "ag.view.prompts"],
  // Skills 管理自旧独立页并入（融合原型回灌）：都是代理工具链管理，其内部页签原样保留。
  ["skills", "ag.view.skills"],
] as const;
type AgentsView = (typeof VIEWS)[number][0];
const VIEW_HINT: Record<AgentsView, string> = {
  agents: "",
  models: "",
  install: "",
  prompts: "",
  skills: "",
};

export function AgentsPage({ openMatrixAt }: { openMatrixAt?: number }) {
  const [view, setView] = useState<AgentsView>("agents");
  // 跨页跳转：后端页只读分配灯 → 直接落进「模型分配全景」矩阵
  // （pulse 计数由 App 维护，重复点击同一页也会再次触发视图切换）。
  useEffect(() => {
    if (openMatrixAt) setView("models");
  }, [openMatrixAt]);
  const [sel, setSel] = useState<string | null>(null);
  const [showCfg, setShowCfg] = useState(false);
  // 配置面板是 fixed 浮层，会盖住主内容：面板打开时内容区按它实际宽度挤压让位
  // （UI-AUDIT-3 P3-3，宽度由 ConfigPanel 上报，折叠竖带也算）。
  const [cfgOccupy, setCfgOccupy] = useState(0);
  const occupyCfg = useCallback((w: number) => setCfgOccupy(w), []);
  // Esc/✕ 关掉面板后焦点归还「配置文件」按钮（键盘可达，与 Radix Dialog 同口径）。
  const cfgTriggerRef = useRef<HTMLButtonElement>(null);
  const closeCfg = useCallback(() => {
    setShowCfg(false);
    requestAnimationFrame(() => cfgTriggerRef.current?.focus());
  }, []);
  // 只显示已安装代理（用户拍板）：关掉则连未检测到的也列出。默认只看装了的。
  const [showAll, setShowAll] = useState(false);
  const qc = useQueryClient();

  // 安装扫描：装没装、版本、配置落点、一键安装命令。手动「重新扫描」才 force 重探。
  const scan = useQuery({
    queryKey: ["agents-scan"],
    queryFn: () => ipc<AgentScan[]>("agents_scan", { force: false }),
  });
  const rescan = () => {
    void qc.invalidateQueries({ queryKey: ["agents-scan"] });
    // force 重探走一次性查询，拿完结果再失效回缓存。
    void ipc("agents_scan", { force: true }).finally(() =>
      qc.invalidateQueries({ queryKey: ["agents-scan"] }),
    );
  };
  const scans = scan.data ?? [];
  // 重新扫描单一形态（审查 P3.2）：左列底 / 矩阵右上 / 安装引导右上共用同一颗钮，
  // 尺寸与禁用态不再各摆各的。
  const rescanBtn = (
    <Button size="sm" onClick={rescan} disabled={scan.isFetching}>
      {scan.isFetching ? t("ag.scanning") : t("ag.rescan")}
    </Button>
  );
  const installedApps = new Set(scans.filter((s) => s.installed).map((s) => s.app));
  const notInstalled = scans.filter((s) => !s.installed);
  // 扫描未就绪（首帧 / 扫描失败）时先全列，别因为还没扫到就把列表清空。
  // 必须声明在 `list` 之前：`list` 的过滤回调里读它，挪到后面就是 TDZ（tsc 不报，
  // 运行时整页炸「Cannot access ... before initialization」）。
  const scanReady = scans.length > 0;

  const matrix = useQuery({
    queryKey: ["matrix"],
    queryFn: () => ipc<MatrixView>("matrix_get"),
  });
  const status = useQuery({
    queryKey: ["takeover"],
    queryFn: () => ipc<TakeoverStatus>("takeover_status"),
  });
  // 启停态（片三）：列源下发 enabled，停用的端进「已停用」分组。
  const cols = useQuery({
    queryKey: ["client-list"],
    queryFn: () => ipc<ClientCol[]>("client_list"),
  });

  // 装了就出现：可接管的文件客户端一律列出门面（copilot）也一样——CLI 装上后它就有
  // 个正常管理位（可见清单摘要 + 速率扩展 + 安装），没装时不占一行。
  // 未装门面唯一的入口仍是模型分配全景矩阵。
  const list = matrix.data?.clients?.filter(
    (c) => !c.facade || showAll || (scanReady && installedApps.has(c.app)),
  );
  // 只显示已安装（除非 showAll）：未装的不进主分组，另走「未检测到」折叠组。
  const visible = (list ?? []).filter((c) => showAll || !scanReady || installedApps.has(c.app));
  // 默认选中首家可见代理；切换代理即收起配置面板（面板始终跟着选中项走）。
  useEffect(() => {
    if (sel == null && visible.length > 0) setSel(visible[0].app);
  }, [visible, sel]);
  useEffect(() => {
    setShowCfg(false);
  }, [sel]);

  const row = status.data?.clients.find((c) => c.app === sel);
  const meta = list?.find((c) => c.app === sel);
  // 门面判定不能只看过滤后的 list：从「未检测到」折叠组点进来的门面也要有自己的详情卡
  // （否则落到「读取不到接管状态」的死路上，想装都装不了）。
  const anyMeta = matrix.data?.clients?.find((c) => c.app === sel);
  // 分组依据：槽位制（claude 六档 / codebuddy 三档 / pi 主+小双槽）进多槽组，清单制进单槽组。
  const multi = visible.filter((c) => c.slots.length > 0);
  const single = visible.filter((c) => c.slots.length === 0);
  // 启停态（片三）：停用的端不进 matrix 列表，改从列源取，供「已停用」分组与详情。
  const off = (cols.data ?? []).filter((c) => !c.enabled);
  const isOff = sel != null && off.some((c) => c.app === sel);
  // 接管残留：停用了但仍处接管态（文件未回滚），列表上打角标。
  const residual = new Set(
    off
      .map((c) => c.app)
      .filter((app) => status.data?.clients.find((x) => x.app === app)?.managed),
  );

  return (
    <div className="flex h-full flex-col gap-3 pt-[var(--content-pad-top)]" style={{ paddingRight: cfgOccupy || undefined }}>
      {/* 子视图切换。UI-16（用户 2026-11-16，附截图）：代理管理视图里 Tab 挪进左列，
          右侧详情上顶一行占满；「显示全部代理 / 重新扫描」移到左列底部。
          其余视图（矩阵/安装引导）仍是顶部一行 Tab。 */}
      {view !== "agents" && (
        <div className="flex flex-wrap items-center gap-2">
          <Tabs
            variant="segmented"
            ariaLabel={t("ag.viewAria")}
            value={view}
            onChange={(id) => setView(id as typeof view)}
            items={VIEWS.map(([k, labelKey]) => ({ id: k, label: t(labelKey + "Short"), title: t(labelKey) }))}
          />
          {VIEW_HINT[view] && <span className="text-sm text-ink-4">{VIEW_HINT[view]}</span>}
          {/* 矩阵视图仍保留「显示全部代理」入口（代理管理视图的入口在左列底部）。 */}
          {view === "models" && (
            <label className="ml-auto flex cursor-pointer items-center gap-1.5 text-sm text-ink-4 select-none">
              <input type="checkbox" checked={showAll} onChange={(e) => setShowAll(e.target.checked)} />
              {t("ag.showAll")}
            </label>
          )}
          {/* 重新扫描入口在非代理管理视图也要在（代理管理视图挪到了左列底部）。
              提示词视图与扫描无关，不摆无关按钮。 */}
          {view !== "prompts" && view !== "skills" && (
            <span className={view === "models" ? "" : "ml-auto"}>{rescanBtn}</span>
          )}
        </div>
      )}

      {view === "agents" ? (
        <div className="flex min-h-0 flex-1 gap-4">
          {/* 左列：Tab → 列表 → 底部两个操作入口 */}
          <div className="flex w-60 shrink-0 flex-col gap-2">
            <Tabs
              variant="segmented"
              ariaLabel={t("ag.viewAria")}
              value={view}
              onChange={(id) => setView(id as typeof view)}
              items={VIEWS.map(([k, labelKey]) => ({ id: k, label: t(labelKey + "Short"), title: t(labelKey) }))}
            />
            <div className="min-h-0 flex-1">
              <AgentList
                multi={multi}
                single={single}
                off={off}
                residual={residual}
                notInstalled={notInstalled}
                sel={sel}
                onSelect={setSel}
                loading={matrix.isLoading}
                listError={matrix.isError ? matrix.error : undefined}
                onRetryList={() => matrix.refetch()}
                colsError={cols.isError ? cols.error : undefined}
                onRetryCols={() => cols.refetch()}
              />
            </div>
            {/* 工具行上方留一个完整节奏位（审查 2.5）：gap-2 的 8px + mt-2 = 16px，
                不再贴着列表底掉出页面节奏。 */}
            <div className="mt-2 flex flex-wrap items-center gap-2 border-t border-slate-800/60 pt-2 text-sm text-ink-4">
              <label className="flex cursor-pointer items-center gap-1.5 select-none">
                <input type="checkbox" checked={showAll} onChange={(e) => setShowAll(e.target.checked)} />
                {t("ag.showAll")}
              </label>
              {rescanBtn}
            </div>
          </div>

          {/* 右：选中代理的详情（上顶一行，占满原入口行高度） */}
          <div className="min-w-0 flex-1 overflow-auto">
            {sel == null ? (
              <Card>
                <p className="text-sm text-ink-4">
                  {matrix.isLoading ? t("common.loading") : t("ag.pickHint")}
                </p>
              </Card>
            ) : anyMeta?.facade ? (
              <FacadeDetail
                meta={anyMeta}
                scan={scans.find((s) => s.app === sel)}
                onAssign={() => setView("models")}
                onInstalled={rescan}
              />
            ) : !row ? (
              <Card>
                {/* 接管状态读取失败 ≠ 该代理查不到：分开措辞，失败要给重试入口 */}
                {status.isError ? (
                  <QueryError inline title={t("ag.takeoverReadFail")} error={status.error} onRetry={() => status.refetch()} />
                ) : (
                  <p className="text-sm text-ink-4">
                    {status.isLoading ? t("ag.takeoverLoading") : t("ag.takeoverNotFound")}
                  </p>
                )}
              </Card>
            ) : (
              <AgentDetail
                row={row}
                meta={meta}
                status={status.data!}
                isOff={isOff}
                onToggleCfg={() => setShowCfg(!showCfg)}
                cfgTriggerRef={cfgTriggerRef}
              />
            )}
          </div>
        </div>
      ) : view === "models" ? (
        <div className="min-h-0 flex-1 overflow-auto">
          {/* 预设配置一键还原卡撤下（用户 2026-11-16「把这个迁移到设置模块中」）：
              设置页「网关运维」里已有同 IPC（preset_status/preset_restore）的入口，
              这里再摆一张就是双入口——直接删，不在设置页加重复卡。 */}
          <ModelMatrix installedApps={showAll || !scanReady ? null : installedApps} />
        </div>
      ) : view === "prompts" ? (
        <div className="min-h-0 flex-1 overflow-auto">
          <PromptsView />
        </div>
      ) : view === "skills" ? (
        // Skills 页自带顶部留白，只小幅上提（-mt-3）；负 content-pad 会与页签行重叠挡点击。
        <div className="min-h-0 flex-1 overflow-auto -mt-3">
          <SkillsPage />
        </div>
      ) : (
        <div className="min-h-0 flex-1 overflow-auto">
          <Card>
            <div className="mb-2 flex items-baseline justify-between">
              <h3 className="text-base font-semibold text-ink-1">{t("ag.installTitle")}</h3>
              <span className="text-sm text-ink-4">{t("ag.installHint")}</span>
            </div>
            {scan.isError ? (
              <QueryError inline title={t("ag.scanFail")} error={scan.error} onRetry={() => scan.refetch()} />
            ) : scans.length === 0 ? (
              <p className="text-sm text-ink-4">{scan.isLoading ? t("ag.scanning") : t("ag.scanEmpty")}</p>
            ) : (
              <InstallPanel scans={scans} onInstalled={() => scan.refetch()} />
            )}
          </Card>
        </div>
      )}

      {/* 配置文件编辑器：右缘浮层，只在代理管理子视图、跟随当前选中代理 */}
      {showCfg && sel && row && view === "agents" && (
        <ConfigPanel app={sel} managed={row.managed} onClose={closeCfg} onOccupy={occupyCfg} />
      )}
    </div>
  );
}
