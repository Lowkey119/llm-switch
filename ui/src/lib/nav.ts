// 类型化导航（计划 4.2）。
//
// 跨页跳转此前靠 window 上的 CustomEvent 广播：事件名是裸字符串，detail 的形状没人管，
// 拼错一个字母或改了字段只会在运行时静默失效 —— 这类 bug 最难查，因为「点了没反应」。
// 这里换成一份可订阅的 {page, params} 状态：跳转就是写一次状态，页面读自己关心的那部分，
// 形状由类型系统兜住，模块本身不依赖 React 生命周期，可以直接单测。
import { useSyncExternalStore } from "react";
import type { PageKey } from "./pages";

/** 跨页跳转携带的参数：每个键都对应一处真实的「跳过去还要告诉目标页什么」。 */
export interface NavParams {
  /** 后端页「控制台」页签：预选某个后端（控制台并入后端页后不再是独立页）。 */
  consoleBackend?: string | null;
  /** 后端页「去控制台」脉冲：自增计数，重复点同一目标也能切回控制台页签。 */
  consolePulse?: number;
  /** 代理页：切到「模型分配全景」。自增计数，保证重复点同一目标也能触发视图切换。 */
  matrix?: number;
  /** 后端页：切到「参数中心」Tab 并定位某后端。 */
  paramsBackend?: string;
}

export interface NavState {
  page: PageKey;
  params: NavParams;
  /** 每次 navigate 自增：同页重复跳转也必须能被下游感知。 */
  seq: number;
}

let state: NavState = { page: "backends", params: {}, seq: 0 };
const listeners = new Set<() => void>();

function emit() {
  listeners.forEach((f) => f());
}

/** 换页并带上参数。同页重复调用也会自增 seq，订阅方据此判断「又来了一次」。 */
export function navigate(page: PageKey, params: NavParams = {}): void {
  state = { page, params, seq: state.seq + 1 };
  emit();
}

/** 只改参数不换页（例如目标页自己消化掉参数后清场）。 */
export function replaceParams(params: NavParams): void {
  state = { ...state, params };
  emit();
}

export function getNav(): NavState {
  return state;
}

/** 订阅导航状态。state 每次变更都是新对象，getSnapshot 因此天然稳定。 */
export function useNav(): NavState {
  return useSyncExternalStore(
    (cb) => {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
    () => state,
  );
}

// ---- 语义化的跳转入口（调用方读起来仍然像「去哪」，而不是「写状态」）----

/** 后端/参数中心的「控制台」入口 → 后端页「控制台」页签并预选该后端（融合后同页切签）。 */
export function gotoConsole(backendName?: string): void {
  navigate("backends", { consoleBackend: backendName ?? null, consolePulse: state.seq + 1 });
}

/** 后端页的只读分配灯 → 代理页「模型分配全景」（矩阵是分配的唯一写入口）。 */
export function gotoMatrix(): void {
  navigate("agents", { matrix: state.seq + 1 });
}
