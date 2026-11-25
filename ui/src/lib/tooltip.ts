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
  t.classList.add("show");
  place(el, t);
}

function hide() {
  host = null;
  tip?.classList.remove("show");
}

/** 主窗与浮窗入口各调一次（每个 webview 是独立文档，各挂各的）。 */
export function initTooltip(): void {
  const on = (e: Event) => {
    const el = (e.target as Element | null)?.closest?.("[data-tip]");
    if (el && el !== host) show(el);
  };
  const off = (e: Event) => {
    const el = (e.target as Element | null)?.closest?.("[data-tip]");
    if (el === host) hide();
  };
  document.addEventListener("pointerover", on);
  document.addEventListener("pointerout", off);
  document.addEventListener("focusin", on);
  document.addEventListener("focusout", off);
  // 滚动/缩放时气泡跟不住锚点，直接收起（再悬停会重出）。
  window.addEventListener("scroll", hide, true);
  window.addEventListener("resize", hide);
}
