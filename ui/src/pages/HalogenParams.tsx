// 引擎参数编辑器：占用日志卡位置的临时视图（用户拍板「切换日志内容区做调参窗口」）。
// 规格/校验/写盘都在 102 的 hal_env.py / gu_env.py（白名单单一真相），这里按 spec 动态渲染表单。
// ch 选择机位（halogen / gufo）；语义承自 hal_dash：空值=移除键跟随镜像默认、密钥 ***=不变。
//
// 版式（修「参数设置区太小、根本没法操作」，三条都是根因级）：
//   1. 表单按内容自然长高，滚动只走外层面板那一条；不再在卡里套一个 flex-1 的内层
//      滚动区——它只分到上半部卡片剩下的零头，26 个键被压成一条看不见的缝。
//   2. 预设那一排的宽度由外层容器决定。控件上叠宽度类会被 inputCls 的 w-full 顶掉
//      （Tailwind 产物里 w-full 在 w-64/w-36 之后），于是每个控件各占一整行。
//   3. 保存/重启做成吸底工具条，表单再长也够得着，不必滚到底才能提交。
import { useEffect, useMemo, useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Button, inputCls, selectCls } from "../components/ui";
import { ConfirmDialog, Risk } from "../components/ConfirmDialog";
import { useToast } from "../components/Toast";
import { ipc, type HalEnv, type HalOp } from "../lib/ipc";

type Confirm = null | { apply: string } | { del: string } | "restart" | "back";

/** 表单值与磁盘真值逐项比对：缺席与空串同义（都表示「移除该键跟随默认」）。 */
function sameValues(a: Record<string, string>, b: Record<string, string>): boolean {
  const keys = new Set([...Object.keys(a), ...Object.keys(b)]);
  for (const k of keys) if ((a[k] ?? "") !== (b[k] ?? "")) return false;
  return true;
}

export function HalogenParams({ ch = "halogen", onBack }: { ch?: string; onBack: () => void }) {
  const qc = useQueryClient();
  const toast = useToast();
  const env = useQuery({
    queryKey: ["hal", ch, "env"],
    queryFn: () => ipc<HalEnv>("halctl_env", { ch }),
  });
  const [vals, setVals] = useState<Record<string, string>>({});
  // base = 最近一次读到的磁盘真值，用来判「有没有未保存的修改」。
  const [base, setBase] = useState<Record<string, string>>({});
  const [presetName, setPresetName] = useState("");
  const [pickPreset, setPickPreset] = useState("");
  const [confirm, setConfirm] = useState<Confirm>(null);
  const [notes, setNotes] = useState("");

  // 同步只在 env.data 变化时发生，但要读当下的 vals/base，所以走 ref 不进依赖数组。
  const cur = useRef({ vals, base });
  cur.current = { vals, base };
  // 预设动作以磁盘为准强制回写：此时盘里已是预设值，本地未保存的编辑本就该让位。
  const forceSync = useRef(false);

  useEffect(() => {
    const next = env.data?.values;
    if (!next) return;
    // 有未保存修改时不拿 refetch 洗掉用户正在敲的格子（保存后回读的是同一份值，不受影响）。
    if (forceSync.current || sameValues(cur.current.vals, cur.current.base)) setVals(next);
    forceSync.current = false;
    setBase(next);
  }, [env.data]);

  const dirty = useMemo(() => !sameValues(vals, base), [vals, base]);

  const after = (r: HalOp) => {
    setNotes(r.output);
    qc.invalidateQueries({ queryKey: ["hal", ch, "env"] });
    qc.invalidateQueries({ queryKey: ["hal", ch, "status"] });
  };
  const save = useMutation({
    mutationFn: () => ipc<HalOp>("halctl_env_save", { ch, values: vals }),
    onSuccess: after,
    onError: (e) => toast.err(`保存参数失败：${e}`),
  });
  const savePreset = useMutation({
    mutationFn: () =>
      ipc<HalOp>("halctl_env_preset_save", { ch, name: presetName.trim(), values: vals }),
    onSuccess: (r) => {
      after(r);
      setPresetName("");
    },
    onError: (e) => toast.err(`保存预设失败：${e}`),
  });
  const delPreset = useMutation({
    mutationFn: () => ipc<HalOp>("halctl_env_preset_del", { ch, name: pickPreset }),
    onSuccess: (r) => {
      after(r);
      setPickPreset("");
    },
    onError: (e) => toast.err(`删除预设失败：${e}`),
  });
  const applyPreset = useMutation({
    mutationFn: (name: string) => ipc<HalOp>("halctl_env_preset_apply", { ch, name }),
    onSuccess: (r) => {
      forceSync.current = true;
      after(r);
    },
    onError: (e) => toast.err(`应用预设失败：${e}`),
  });
  const restart = useMutation({
    mutationFn: () => ipc<HalOp>("halctl_control", { ch, action: "restart" }),
    onSuccess: after,
    onError: (e) => toast.err(`重启失败：${e}`),
  });

  const busy =
    save.isPending || savePreset.isPending || delPreset.isPending || applyPreset.isPending || restart.isPending;
  const busyText = applyPreset.isPending
    ? "应用预设中（写参数 + 重启等待就绪，约 1~5 分钟）…"
    : restart.isPending
      ? "重启中（等待引擎就绪）…"
      : busy
        ? "提交中…"
        : "";

  // 收起时若还有未保存修改，先问一句：这个视图是卸载式的，直接切走编辑就没了。
  const goBack = () => (dirty ? setConfirm("back") : onBack());

  const runConfirm = () => {
    if (confirm === "restart") restart.mutate();
    else if (confirm === "back") onBack();
    else if (confirm && typeof confirm === "object" && "del" in confirm) delPreset.mutate();
    else if (confirm && typeof confirm === "object") applyPreset.mutate(confirm.apply);
  };

  const spec = env.data?.spec ?? [];
  const presets = env.data?.presets ?? [];

  // 文案里的引擎名/参数文件名随通道走（gufo → gufo.conf）
  const engName = ch === "gufo" ? "gufo" : "halogen";
  const paramFile = ch === "gufo" ? "gufo.conf" : "halogen.env";

  return (
    <>
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs">
        <span className="font-medium text-ink-2">参数设置</span>
        <span className="text-xs text-ink-4">空值 = 移除该键跟随镜像默认</span>
        {dirty && <span className="text-xs text-[var(--sem-busy-text)]">有未保存的修改</span>}
        <div className="ml-auto">
          <Button onClick={goBack}>返回日志</Button>
        </div>
      </div>

      {/* 预设行：一键应用 | 自定义命名保存/删除。宽度套在外层容器上，窄屏各自折行。 */}
      <div className="flex flex-wrap items-center gap-2 text-xs">
        <div className="w-full sm:w-64">
          <select className={selectCls} value={pickPreset} onChange={(e) => setPickPreset(e.target.value)}>
            <option value="">选择预设…</option>
            {presets.map((p) => (
              <option key={p.name} value={p.name}>
                {p.name}（{p.when}，{p.n} 键）
              </option>
            ))}
          </select>
        </div>
        <Button
          variant="primary"
          disabled={busy || !pickPreset}
          onClick={() => setConfirm({ apply: pickPreset })}
        >
          应用预设
        </Button>
        <span className="hidden h-5 w-px bg-slate-700 sm:block" aria-hidden />
        <div className="w-full sm:w-40">
          <input
            className={inputCls}
            placeholder="新预设名"
            value={presetName}
            onChange={(e) => setPresetName(e.target.value)}
          />
        </div>
        <Button disabled={busy || !presetName.trim()} onClick={() => savePreset.mutate()}>
          存为预设
        </Button>
        <Button variant="danger" disabled={busy || !pickPreset} onClick={() => setConfirm({ del: pickPreset })}>
          删除预设
        </Button>
      </div>

      {env.isError && <div className="text-xs text-[var(--sem-danger-text)]">参数加载失败：{String(env.error)}</div>}
      {env.isLoading && <div className="text-xs text-ink-4">读取 {paramFile}…</div>}

      {/* 分组表单：规格来自 102（hal_env.py spec），前端不写死字段。
          字段一律「标签在上、控件在下」堆叠 + min-w-0：横排时标签和控件的固有宽会撑爆
          栅格列（实报横向溢出没法操作），列宽必须可收缩。 */}
      <div className="space-y-4">
        {spec.map((g) => (
          <div key={g.group}>
            <div className="mb-1.5 text-xs font-medium text-cyan-300/80">{g.group}</div>
            <div className="grid grid-cols-1 gap-x-4 gap-y-2.5 sm:grid-cols-2 xl:grid-cols-3">
              {g.items.map((it) => (
                <label key={it.key} className="flex min-w-0 flex-col gap-1">
                  <span className="truncate text-xs text-ink-3" title={it.key}>
                    {it.label}
                  </span>
                  {it.kind === "opt" ? (
                    <select
                      className={selectCls}
                      value={vals[it.key] ?? ""}
                      onChange={(e) => setVals({ ...vals, [it.key]: e.target.value })}
                    >
                      <option value="">（默认）</option>
                      {(it.opts ?? []).map((o) => (
                        <option key={o} value={o}>
                          {o}
                        </option>
                      ))}
                    </select>
                  ) : (
                    <input
                      className={inputCls}
                      type={it.kind === "key" ? "password" : "text"}
                      inputMode={it.kind === "num" ? "decimal" : undefined}
                      placeholder={
                        it.kind === "key" ? "***=不变，-=清除" : it.kind === "num" ? "空=默认" : "空=移除"
                      }
                      value={vals[it.key] ?? ""}
                      onChange={(e) => setVals({ ...vals, [it.key]: e.target.value })}
                    />
                  )}
                </label>
              ))}
            </div>
          </div>
        ))}
      </div>

      {notes && (
        <pre className="max-h-24 overflow-auto whitespace-pre-wrap rounded-md border border-slate-800 bg-slate-950/60 p-2 text-xs leading-relaxed text-ink-3">
          {notes}
        </pre>
      )}

      {/* 吸底工具条：负 margin 抹平卡片内边距让它贴齐卡片两边。 */}
      <div className="sticky bottom-0 -mx-4 -mb-3 flex flex-wrap items-center gap-2 border-t border-slate-800 bg-slate-900/95 px-4 py-2 backdrop-blur-sm">
        <Button variant="primary" disabled={busy || !env.data || !dirty} onClick={() => save.mutate()}>
          保存参数
        </Button>
        <Button disabled={busy || !dirty} onClick={() => setVals(base)}>
          放弃修改
        </Button>
        <Button disabled={busy} onClick={() => setConfirm("restart")}>
          立即重启生效
        </Button>
        {busyText && <span className="text-xs text-[var(--sem-busy-text)]">{busyText}</span>}
      </div>

      <ConfirmDialog
        open={confirm != null}
        title={
          confirm === "restart"
            ? `重启 ${engName}？`
            : confirm === "back"
              ? "放弃未保存的修改？"
              : confirm && typeof confirm === "object" && "del" in confirm
                ? "删除预设？"
                : "应用预设？"
        }
        danger={confirm !== "back"}
        confirmLabel="确认"
        onClose={() => setConfirm(null)}
        onConfirm={runConfirm}
      >
        {confirm === "restart" ? (
          <>这会<Risk>中断所有在飞请求</Risk>，重启需等待引擎重新加载权重（约 1~5 分钟）。</>
        ) : confirm === "back" ? (
          <>当前表单有<b>未保存的修改</b>，<Risk>返回日志会丢弃这些改动</Risk>。</>
        ) : confirm && typeof confirm === "object" && "del" in confirm ? (
          <>
            将删除预设 <b>{confirm.del}</b>，<Risk>删除后不可恢复</Risk>。
          </>
        ) : confirm && typeof confirm === "object" ? (
          <>
            将把预设 <b>{confirm.apply}</b> 写入 {paramFile} 并<Risk>重启引擎（中断在飞请求）</Risk>。
            预设不含 API 密钥，密钥保持现状。
            {dirty && <> 当前表单有未保存的修改，应用预设会以预设值覆盖它们。</>}
          </>
        ) : null}
      </ConfirmDialog>
    </>
  );
}
