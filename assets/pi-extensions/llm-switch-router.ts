/**
 * llm-switch-router — 读 `piSlots` 的大小模型路由（llm-switch 双槽的消费侧）
 * --------------------------------------------------------------------------
 * llm-switch 的模型分配矩阵给 pi 写两个互斥槽位（models.json provider 节点）：
 *
 *   providers.llm-switch.piSlots = { "main": "<主模型全id>", "small": "<小模型全id>" }
 *
 * pi 本体不读这个标记——pi 的「虚拟模型」路由器要由扩展注册（docs/virtual-models.md）。
 * 本扩展注册 `llm-switch/auto` 虚拟模型，每个请求现读 piSlots 分派物理模型：
 *
 *   - 你发的新消息（reason=user）：档位 high → 主模型；档位 low → 小模型
 *     （其它档位一律走主模型——硬任务不能被静默降级到小模型）
 *   - 工具续跑 / 失败重试（continuation / retry）：钉住上一物理模型，
 *     保住 prompt cache 与思考签名（官方 jev-router 同款策略）
 *   - 压缩摘要等直调（direct）：走小模型省 token
 *   - 槽位缺配时退化到另一个槽；两槽全空则报错指路
 *
 * 每请求现读（几 KB 同步读，相对 LLM 请求可忽略），因此在 llm-switch 里换槽
 * **下一次请求立即生效**，无需重启 pi；解析失败退回上一次成功读到的槽值。
 *
 * 用法：pi 里 `/model` 选 `llm-switch/auto`（或把 settings.json 的 defaultModel
 * 设成 "auto" 让它当默认）；页脚实时显示路由结果，例如：
 *
 *   auto • low → ornith-1.5-35b-a3b-mtp-apex
 *   auto • high → qwen3.8-flash-next
 *
 * 源码由 llm-switch 仓统一管理（assets/pi-extensions/llm-switch-router.ts），随包内嵌，
 * 经「代理 → pi → 扩展」一键安装进 ~/.pi/agent/extensions/（与 gen-rate.ts 同惯例）。
 * 仅类型导入 = 运行时不依赖 pi 包，可独立做 mock 自测。
 */

import type { ExtensionAPI, ExtensionContext, ModelRoute, ModelRouteRequest } from "@earendil-works/pi-coding-agent";
import { readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

/** pi 清单里 llm-switch 的 provider id（llm-switch 接管写入的那个节点）。 */
const PROVIDER = "llm-switch";
/** 虚拟模型 id：不得与 provider 下任何物理模型同名（"auto" 是 pi 社区路由惯例名）。 */
const VIRTUAL_ID = "auto";
/** pi 的配置目录（对齐 pi 自身的 PI_CODING_AGENT_DIR，见 pi config.d.ts ENV_AGENT_DIR）。 */
const AGENT_DIR = process.env.PI_CODING_AGENT_DIR ?? join(homedir(), ".pi", "agent");

interface Slots {
  main?: string;
  small?: string;
}

/** 上一次成功解析的槽值：只兜「读不到/半截 JSON」，不代表配置真值。 */
let lastGood: Slots = {};

/** 现读 piSlots 标记。只有**文件读不出或解析失败**（正被重写等瞬时态）才退回上次值；
 * 解析成功说明文件完整——槽位被清掉就是真值为空，必须停用，不能拿旧值装活。 */
function readSlots(): Slots {
  let doc: unknown;
  try {
    doc = JSON.parse(readFileSync(join(AGENT_DIR, "models.json"), "utf8"));
  } catch {
    return lastGood;
  }
  const slots = (doc as { providers?: Record<string, { piSlots?: Record<string, unknown> }> })
    ?.providers?.[PROVIDER]?.piSlots;
  const main = typeof slots?.main === "string" ? slots.main : undefined;
  const small = typeof slots?.small === "string" ? slots.small : undefined;
  lastGood = main || small ? { main, small } : {};
  return lastGood;
}

/** 选槽：目标槽空则退到另一槽；两槽全空报错指路（用户可见的配置错误要说清去哪修）。 */
function pickSlot(slots: Slots, wantSmall: boolean): string {
  const id = (wantSmall ? slots.small : slots.main) ?? (wantSmall ? slots.main : slots.small);
  if (!id) throw new Error("llm-switch 的 piSlots 主/小槽都没配——先在 llm-switch「模型分配矩阵」里给 pi 分配");
  return id;
}

export default function (pi: ExtensionAPI): void {
  pi.registerVirtualModel({
    provider: PROVIDER,
    id: VIRTUAL_ID,
    name: "Auto · 主/小路由",
    // 档位是给路由器看的输入，不是推理预算：low=走小模型，high=走主模型
    thinkingLevels: ["low", "high"],
    route(request: ModelRouteRequest, ctx: ExtensionContext): ModelRoute {
      // 1) 续跑/重试钉住上一物理模型（保 prompt cache 与思考签名）
      const sticky = request.failed ?? request.previous;
      if (request.reason !== "user" && sticky) {
        return { model: sticky.model, thinkingLevel: sticky.thinkingLevel ?? "off" };
      }
      // 2) direct（压缩摘要、扩展直调）走小模型省 token；
      //    user 只有显式 low 才走小模型，其余（high/未知档）一律走主模型
      const wantSmall = request.reason === "direct" || request.thinkingLevel === "low";
      const slots = readSlots();
      const id = pickSlot(slots, wantSmall);
      const model = ctx.modelRegistry.find(PROVIDER, id);
      if (!model) {
        throw new Error(`piSlots 指向的模型 ${PROVIDER}/${id} 不在清单里——去 llm-switch「模型分配矩阵」重配`);
      }
      // 物理模型只支持的档位由 pi 自行钳制（llm-switch 的模型不声明 reasoning → 钳到 off）
      return { model, thinkingLevel: request.thinkingLevel };
    },
  });
}
