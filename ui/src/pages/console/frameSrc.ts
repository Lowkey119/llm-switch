// 控制台 iframe URL 计算（纯函数，从 ConsolePage 原样搬出）。
import type { BackendRow, ShellStatus } from "../../lib/ipc";
import type { Theme } from "../../lib/theme";

/** 监听地址 → 网关 origin：0.0.0.0/::/* 折回 127.0.0.1（本机自访问）。 */
export function gatewayOrigin(listen: string): string {
  const i = listen.lastIndexOf(":");
  if (i < 0) return "";
  let host = listen.slice(0, i);
  if (host === "0.0.0.0" || host === "::" || host === "*" || host === "[::]") host = "127.0.0.1";
  if (!host) return "";
  return `http://${host}:${listen.slice(i + 1)}`;
}

/** iframe 地址 + 「新窗口」按钮共用：
 *  无尾斜杠：网关的 matchit 路由不匹配 "/console/:id/"（空尾路径 404，见 routes.rs M7 注释）。
 *  带令牌：iframe 发不出 Authorization 头，走 gateway/auth.rs 为 /console/* 开的
 *  query 通道（同源子资源靠 Referer / Cookie 自动续上）。
 *  `?theme=` 管首屏配色：postMessage 要等文档加载完才收得到，先来一次才不闪黑。
 *  运行时切换再走 postMessage（ConsoleFrame 的主题跟随 effect）。两个通道缺一不可——
 *  「新窗口」打开的面板没有父页面可发消息，而重载又会丢掉运行时改的那个。 */
export function buildConsoleUrl(
  sel: Pick<BackendRow, "id">,
  gatewayBase: string,
  status: Pick<ShellStatus, "access_token"> | undefined,
  theme: Theme,
): string {
  if (!gatewayBase) return "";
  const token = status?.access_token;
  return `${gatewayBase}/console/${sel.id}?${[
    token ? `access_token=${encodeURIComponent(token)}` : "",
    `theme=${theme}`,
  ]
    .filter(Boolean)
    .join("&")}`;
}
