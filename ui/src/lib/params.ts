// 参数体系的共享件：策略选项、内置模板、以及「继承链最终值」推算。
// 抽出来的原因是模型清单的铅笔弹窗和页面下方的参数区必须算出同一个答案。

export interface ParamRow {
  id: number;
  scope: string;
  ref_id: string;
  key: string;
  value: unknown;
  policy: string;
}

export const POLICIES = [
  { v: "fill", label: "填充（客户端没给才用）" },
  { v: "override", label: "强制声明（同填充，历史兼容）" },
  { v: "clamp", label: '钳制区间 {"min":…, "max":…}' },
];

export const TEMPLATES: { name: string; params: [string, string, string][] }[] = [
  {
    name: "coder",
    params: [
      ["temperature", "0.2", "fill"],
      ["top_p", "0.9", "fill"],
      ["repetition_penalty", "1.05", "fill"],
    ],
  },
  {
    name: "chat",
    params: [
      ["temperature", "0.7", "fill"],
      ["top_p", "0.95", "fill"],
    ],
  },
  {
    name: "creative",
    params: [
      ["temperature", "1.1", "fill"],
      ["frequency_penalty", "0.3", "fill"],
      ["presence_penalty", "0.3", "fill"],
    ],
  },
];

export interface ChainLink {
  key: string;
  global?: ParamRow;
  backend?: ParamRow;
  model?: ParamRow;
  /** 最终会生效的那一层：模型 → 全局 → 后端，第一个活着的层；全为 0=继承时是 inherit。 */
  winner: "model" | "global" | "backend" | "inherit";
  winner_value: unknown;
  winner_policy: string;
}

/**
 * 把一个模型作用域里出现过的参数键，按 请求 > 模型 > 全局 > 后端 摊成继承链，
 * 与后端 `params::apply` 同口径（片2 定案：层间先到先得，后端层兜底）。
 * 回答的是「客户端没传这个参数时，最终会用谁的值」：
 * - 长度量纲键（lengthKeys）配 0 = 该层继承，不参与竞争；
 * - 全部层都是 0 → winner=inherit（不下发，走后端默认）；
 * - clamp 只提供区间，这里按「有配就算命中该层」呈现，具体夹取在策略说明里。
 */
export function buildChains(
  rows: ParamRow[],
  backend: string,
  model: string,
  lengthKeys: ReadonlySet<string>,
): ChainLink[] {
  const at = (scope: string, ref: string, key: string) =>
    rows.find((r) => r.scope === scope && r.ref_id === ref && r.key === key);
  /** 活着的层：长度键配 0 = 继承，不算命中。 */
  const alive = (r?: ParamRow) =>
    r && !(lengthKeys.has(r.key) && r.value === 0) ? r : undefined;
  const modelRef = `${backend}:${model}`;
  const keys = Array.from(
    new Set(
      rows
        .filter(
          (r) =>
            r.scope === "global" ||
            (r.scope === "backend" && r.ref_id === backend) ||
            (r.scope === "model" && r.ref_id === modelRef),
        )
        .map((r) => r.key),
    ),
  ).sort();

  return keys.flatMap((key) => {
    const g = at("global", "", key);
    const b = at("backend", backend, key);
    const m = at("model", modelRef, key);
    if (!g && !b && !m) return [];
    const win = alive(m) ?? alive(g) ?? alive(b);
    const winner: ChainLink["winner"] = !win
      ? "inherit"
      : win === m
        ? "model"
        : win === g
          ? "global"
          : "backend";
    return [
      {
        key,
        global: g,
        backend: b,
        model: m,
        winner,
        winner_value: win?.value,
        winner_policy: win?.policy ?? "",
      } as ChainLink,
    ];
  });
}

export const WINNER_LABEL: Record<ChainLink["winner"], string> = {
  model: "模型覆盖",
  global: "全局默认",
  backend: "后端默认",
  inherit: "继承（0=未配）",
};

/**
 * 参数下发战报（后端 `services::param_push::Outcome::to_value` 的形状）。
 * 模型层保存/删除时后端会顺手把参数写进后端本体的模型配置；
 * 非模型层或后端不存在时为 null（没有下发这回事）。
 */
export interface PushReport {
  channel: string;
  ok: boolean;
  pushed: string[];
  cleared: string[];
  unsupported: string[];
  restart_required: boolean;
  detail: string;
}

/**
 * 把下发战报翻译成人话（返回 toast 档位 + 文案；null = 非模型层，维持原话术）。
 * 口径：写后端失败 ≠ 保存失败——库里已经存了，请求注入照常生效，
 * 所以用 error 说清「已保存但没写进后端」，别谎报成功也别吓人说保存失败。
 * 后端 detail 是按「写入」措辞的，删除（清除）时换成自己的说法。
 */
export function pushReportToast(
  r: PushReport | null,
  action: "save" | "delete",
  key: string,
): { kind: "ok" | "info" | "err"; msg: string } | null {
  if (!r) return null;
  const extra =
    r.unsupported.length > 0
      ? `；${r.unsupported.join("、")} 该后端不认，仅对经网关的请求生效`
      : "";
  if (!r.ok)
    return {
      kind: "err",
      msg: `参数中心已${action === "save" ? "保存" : "删除"}，但写入后端失败：${r.detail}`,
    };
  if (r.channel === "none") return { kind: "info", msg: `${r.detail}${extra}` };
  if (r.pushed.length + r.cleared.length === 0)
    return { kind: "info", msg: `${r.detail}${extra}` };
  const msg =
    action === "save"
      ? r.detail
      : `已清除后端里 ${key} 的配置，恢复其默认${
          r.restart_required ? "（引擎级参数，重启后生效）" : ""
        }`;
  return { kind: r.restart_required ? "info" : "ok", msg: `${msg}${extra}` };
}

/** useToast 的最小面（lib 不引组件，接口对齐即可）。 */
export interface ToastLike {
  ok(msg: string): void;
  err(msg: string): void;
  info(msg: string): void;
}

/**
 * 战报上屏：有战报说战报，没战报（非模型层）走 fallback。
 * 铅笔弹窗与参数中心共用这一条口径，两处话术不许分叉。
 */
export function applyPushReport(
  toast: ToastLike,
  r: PushReport | null,
  action: "save" | "delete",
  key: string,
  fallback: () => void,
): void {
  const t = pushReportToast(r, action, key);
  if (!t) return fallback();
  if (t.kind === "ok") toast.ok(t.msg);
  else if (t.kind === "err") toast.err(t.msg);
  else toast.info(t.msg);
}
