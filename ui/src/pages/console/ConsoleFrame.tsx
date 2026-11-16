// 控制台 iframe 帧（从 ConsolePage 原样搬出）：内嵌引擎面板 + 加载失败覆盖层。
// 持有 iframe 自己的本地态：加载态、反缩放订阅、主题 postMessage——
// 「刷新 / 保存 / 起停」的重载信号由页层 frameKey 传入。
import { useEffect, useRef, useState } from "react";
import { subscribeZoom, zoomFactor } from "../../lib/zoom";
import { Button } from "../../components/ui";
import type { BackendRow } from "../../lib/ipc";
import type { Theme } from "../../lib/theme";
import { buildConsoleUrl } from "./frameSrc";

export function ConsoleFrame({
  sel,
  gatewayBase,
  token,
  theme,
  frameKey,
  onReload,
  onEdit,
}: {
  sel: BackendRow;
  gatewayBase: string;
  token?: string | null;
  theme: Theme;
  /** 页层重载信号（工具条「刷新」、保存成功、起停脚本之后都会 +1）。 */
  frameKey: number;
  /** 覆盖层「重试」：页层 frameKey +1。 */
  onReload: () => void;
  /** 覆盖层「入口设置」。 */
  onEdit: () => void;
}) {
  const frameRef = useRef<HTMLIFrameElement>(null);
  // iframe 加载态（任务 3.5）：跨域 iframe 里上游挂了不会冒到父页面，onLoad
  // 也可能压根不触发，所以超时兜底是必须的——否则上游没起来就是一块永久空白。
  const [frameState, setFrameState] = useState<"loading" | "ok" | "fail">("loading");
  // 主窗缩放倍率（lib/zoom.ts）。iframe 反缩放要用：WebKit 在被 CSS zoom 的文档里
  // 会把 iframe 内视口按 zoom 后的视觉宽计算、内容又按倍率渲染 → 净溢出（详见下方 effect）。
  const [zoom, setZoom] = useState(zoomFactor);

  // 无尾斜杠 + 令牌 + ?theme= 的完整口径见 frameSrc.ts。
  const frameSrc = buildConsoleUrl(sel, gatewayBase, { access_token: token }, theme);

  useEffect(() => {
    setFrameState("loading");
    const t = window.setTimeout(
      () => setFrameState((s) => (s === "loading" ? "fail" : s)),
      15000,
    );
    return () => window.clearTimeout(t);
  }, [frameKey, frameSrc]);

  // 主题第二条通道：运行时切换不重载 iframe 也得跟上——重载会把面板的滚动位置与
  // 未保存的输入一并冲掉。targetOrigin 取网关 origin：Tauri 的 UI 与 iframe 不同源，
  // 写 "*" 会让任何嵌进来的页面都收到这条消息。
  useEffect(() => {
    if (!gatewayBase) return; // 目标 origin 为空时 postMessage 会直接抛 SyntaxError
    frameRef.current?.contentWindow?.postMessage({ type: "llms-theme", theme }, gatewayBase);
  }, [theme, frameKey, frameSrc, gatewayBase]);

  // iframe 反缩放（**真·根因修复**）：主窗把 CSS zoom 挂在 documentElement 上（lib/zoom.ts），
  // 而 WebKit 对 zoom 树里的 iframe 会把**内视口按 zoom 后的视觉宽**计算（实测 zoom=1.33、
  // iframe 布局 706：内层 clientWidth=928 ≈ 706×1.33−滚动条，visualViewport 却只有 ~697），
  // 内容于是按 928 布局塞进 ~697 的视口 → 恒溢出 1.33 倍（横向滚动条 + 右裁切，三张实拍
  // 的病灶）；viewport meta 定宽救不了（实测 width=706 同样被覆盖成 928）。解法与浮窗圆钮
  // 同款（FloatPanel：`zoom: 1/zoom`，legacy zoom 必须 JS 写**数字**，calc() 在 WebKit 里
  // 无效）：iframe 自身反缩放后，内视口 == 面板视觉宽（实测 clientW ≈ visualViewport ≈ 939，
  // dpr 回到真实值），内容 1:1 按屏幕像素响应式布局，缩放窗口实时跟随、零重载。
  useEffect(() => {
    setZoom(zoomFactor()); // 订阅前对齐一次：App 的 useZoom 可能晚于本组件应用倍率
    return subscribeZoom(setZoom);
  }, []);

  // 已连接后不再自动重载 iframe（用户口径）：内嵌面板连上后，定时重载 / 切回窗口重载
  // 会冲掉用户在面板里的滚动位置与未保存输入、打断操作。需要刷新用工具条「刷新」手动触发。

  return (
    <div className="relative flex min-h-0 flex-1">
      <iframe
        key={frameKey}
        ref={frameRef}
        src={frameSrc}
        onLoad={() => {
          setFrameState("ok");
          // 加载完再补一次主题：文档没起来之前发的消息会被丢掉，
          // 而 `?theme=` 只在进这一帧时算过一次。
          if (gatewayBase) {
            frameRef.current?.contentWindow?.postMessage({ type: "llms-theme", theme }, gatewayBase);
          }
        }}
        // 底色跟主题走：上游面板还没画完时，不再是一块刺眼的白。
        className="min-h-0 flex-1 rounded-lg border border-slate-800 bg-slate-900"
        // 反缩放：抵消主窗 documentElement 的 CSS zoom（原因见上方 effect 注释）。
        style={{ zoom: 1 / zoom }}
        sandbox="allow-scripts allow-same-origin allow-forms allow-popups"
        title={`${sel.name} 控制台`}
      />
      {frameState !== "ok" && (
        <div className="absolute inset-0 flex flex-col items-center justify-center gap-2 rounded-lg bg-slate-900 text-xs text-ink-3">
          {frameState === "loading" ? (
            <>
              <span className="inline-block h-4 w-4 animate-spin rounded-full border-2 border-slate-700 border-t-cyan-400" />
              正在打开 {sel.name} 的控制台…
            </>
          ) : (
            <>
              {/* 连不上不是错误（BUG-7，用户 2026-11-16）：入口地址没配 / 这家压根
                  不提供 WebUI 都是常态配置，红字告警只会让人以为后端坏了。中性占位 +
                  可动的按钮，语气与「正在打开…」同级。 */}
              <span className="text-ink-3">
                {sel.name} 的控制台没有响应：可能上游引擎没起来、入口地址没配，
                或这家后端不提供 WebUI 控制台。
              </span>
              <span className="text-ink-4">
                可核对入口配置；确实没有 WebUI 就直接用后端原生控制台，不必绕这一层。
              </span>
              <span className="mt-1 flex flex-wrap items-center justify-center gap-2">
                <Button
                  onClick={() => {
                    setFrameState("loading");
                    onReload();
                  }}
                >
                  重试
                </Button>
                <Button onClick={onEdit}>入口设置</Button>
                <Button onClick={() => window.open(sel.webui_url || sel.base_url, "_blank")}>
                  打开后端原生地址
                </Button>
              </span>
            </>
          )}
        </div>
      )}
    </div>
  );
}
