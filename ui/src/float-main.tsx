// 浮窗入口（M12-C2a）：与主窗分开的一个 HTML 入口，共用同一份样式与 IPC 层。
//
// 同一个入口也服务速率线窗口（标签 `rate-<app>`）：Tauri 不吃带查询串的 URL，
// 没法靠 ?app=pi 分流，就用窗口标签当归属的唯一真值。标签在本地就能读到
// （getCurrentWindow 不发 IPC），不必为此多开一条 IPC，也不给启动加一次往返。
import React from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import ReactDOM from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import FloatPanel from "./components/FloatPanel";
import { RateLineWindow } from "./components/FloatRateLine";
import SysFloatWindow from "./components/SysFloat";
// 副作用导入：上色 + 挂 storage 监听，主窗切主题时浮窗实时跟随。
import "./lib/theme";
import "./styles.css";

document.documentElement.classList.add("float-view");

const queryClient = new QueryClient({
  defaultOptions: { queries: { refetchOnWindowFocus: false, retry: 1 } },
});

// 速率线窗口标签的约定前缀，与 Rust 侧 rateline::LABEL_PREFIX 同值。
const RATE_PREFIX = "rate-";
/// 资源小窗标签，与 Rust 侧 sysfloat::LABEL 同值。
const SYSFLOAT_LABEL = "sysfloat";

/** 本窗口是哪一家的速率线；普通浮窗返回 null。 */
function rateApp(): string | null {
  try {
    const label = getCurrentWindow().label;
    return label.startsWith(RATE_PREFIX) ? label.slice(RATE_PREFIX.length) : null;
  } catch {
    return null;
  }
}

function Root() {
  const app = rateApp();
  if (app) return <RateLineWindow app={app} />;
  try {
    if (getCurrentWindow().label === SYSFLOAT_LABEL) return <SysFloatWindow />;
  } catch {
    /* 读不到标签就当普通浮窗 */
  }
  return <FloatPanel />;
}

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <QueryClientProvider client={queryClient}>
      <Root />
    </QueryClientProvider>
  </React.StrictMode>,
);
