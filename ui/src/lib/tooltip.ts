// 全局 tooltip（审查 2026-11-25 P2 收口）：替代原先 .tip::after 的逐元素 CSS 气泡。
//
// 为什么要换成单一 fixed 浮层：CSS ::after 画在触发元素自己的层里，逃不出
// overflow:hidden / overflow-y:hidden 的容器（页面整页不竖滚、卡片裁边都会吞气泡），
// 也量不到自身宽高做翻转——此前只能逐点打 tip-below / tip-up / tip-edge 补丁。
// 单一浮层挂 body 下、fixed 定位，先下后上自动翻转 + 水平钳进视口，一处解决全部。
//
// 调用点零改动：仍读 [data-tip] 属性（.tip 类退化为纯标记）。
// zoom 口径：fixed 的 left/top 走 zoom 树（本地像素），getBoundingClientRect 给视觉像素，
// 统一经 viewportToLocal 换算（法条：换算只许在 zoom.ts 一处）。

import { viewportToLocal, zoomFactor } from "./zoom";

const PAD = 8;
const GAP = 6;

let tip: HTMLDivElement | null = null;
let host: Element | null = null;
/** 当前气泡显示的文案：用来认出「锚点还在、但已经不是当初那一条」。 */
let shownText = "";
let watcher: MutationObserver | null = null;

function ensureTip(): HTMLDivElement {
  if (tip) return tip;
  tip = document.createElement("div");
  tip.className = "global-tip";
  tip.setAttribute("aria-hidden", "true");
  document.body.appendChild(tip);
  return tip;
}

function place(el: Element, t: HTMLDivElement) {
  const z = zoomFactor() || 1;
  const r = el.getBoundingClientRect();
  // 先落文案再量：宽高随文案变，必须在同一帧里量完再定位。
  const tw = t.offsetWidth;
  const th = t.offsetHeight;
  const vw = window.innerWidth / z;
  const vh = window.innerHeight / z;
  const top = viewportToLocal(r.left + r.width / 2, r.bottom + GAP);
  let x = top.x - tw / 2;
  let y = top.y;
  // 下方放不下翻到上方（tip-up / tip-below 补丁的通用化）。
  if (y + th + PAD > vh) {
    const above = viewportToLocal(r.left + r.width / 2, r.top - GAP);
    y = above.y - th;
  }
  if (y < PAD) y = PAD;
  // 水平钳进视口（tip-edge 补丁的通用化）。
  x = Math.min(Math.max(PAD, x), Math.max(PAD, vw - tw - PAD));
  t.style.left = `${x}px`;
  t.style.top = `${y}px`;
}

function show(el: Element) {
  const text = el.getAttribute("data-tip");
  if (!text) return;
  const t = ensureTip();
  t.textContent = text;
  host = el;
  shownText = text;
  t.classList.add("show");
  place(el, t);
  watchAnchor();
}

function hide() {
  watcher?.disconnect();
  watcher = null;
  host = null;
  tip?.classList.remove("show");
}

/**
 * 气泡显示期间盯 DOM：锚点掉出文档、或它身上的文案已经不是当初那一条，立刻收。
 *
 * 为什么非盯着 DOM 不可：React 把正悬停的那块整块换掉时（加完一条动作，芯片行跟着重排），
 * 浏览器既不补发 pointerout，鼠标也没动所以 pointermove 也不会来——上一版只加指针事件
 * 兜底就是这么漏的（用户 2026-11-25 实报「白色 tooltip 不消失」，且已在 v0.2.116 内仍复现）。
 * 只在气泡可见期间观察，一收就 disconnect，不给全页挂常驻开销。
 */
function watchAnchor() {
  if (!watcher) {
    watcher = new MutationObserver(() => {
      if (!host || !host.isConnected || host.getAttribute("data-tip") !== shownText) hide();
    });
  }
  watcher.observe(document.body, { childList: true, subtree: true });
}

/** 主窗与浮窗入口各调一次（每个 webview 是独立文档，各挂各的）。 */
export function initTooltip(): void {
  const on = (e: Event) => {
    const el = (e.target as Element | null)?.closest?.("[data-tip]");
    // 指针落到不带提示的元素上也要收：只靠 pointerout 会漏——React 把正悬停的那个
    // 节点整块换掉时（加完一条条目，芯片行跟着重排）浏览器不再补发 pointerout，
    // 气泡就永远挂在原地（用户 2026-11-25 实报「白色 tooltip 不消失」）。
    if (!el) {
      if (host) hide();
      return;
    }
    if (el !== host) show(el);
  };
  const off = (e: Event) => {
    const el = (e.target as Element | null)?.closest?.("[data-tip]");
    if (el === host) hide();
  };
  // 兜底：锚点被重渲染摘掉（isConnected=false）后，任何一次指针移动都顺手收掉，
  // 免得非要有 pointerout 才收得到。
  const sweep = () => {
    if (host && !host.isConnected) hide();
  };
  document.addEventListener("pointerover", on);
  document.addEventListener("pointerout", off);
  document.addEventListener("pointermove", sweep);
  document.addEventListener("focusin", on);
  document.addEventListener("focusout", off);
  // 滚动/缩放时气泡跟不住锚点，直接收起（再悬停会重出）。
  window.addEventListener("scroll", hide, true);
  window.addEventListener("resize", hide);
}
