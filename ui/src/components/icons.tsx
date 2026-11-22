import type { SVGProps } from "react";

const base = (props: SVGProps<SVGSVGElement>) => ({
  width: 18,
  height: 18,
  viewBox: "0 0 24 24",
  fill: "none",
  stroke: "currentColor",
  strokeWidth: 1.7,
  strokeLinecap: "round" as const,
  strokeLinejoin: "round" as const,
  ...props,
});

export const IconBackend = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <rect x="3" y="4" width="18" height="6" rx="1.5" />
    <rect x="3" y="14" width="18" height="6" rx="1.5" />
    <path d="M7 7h.01M7 17h.01" />
  </svg>
);

/** 横置电池：level 0~1 决定内部填充宽度，颜色由调用方 currentColor 决定。 */
export const IconBattery = ({ level = 1, ...p }: SVGProps<SVGSVGElement> & { level?: number }) => (
  <svg {...base(p)}>
    <rect x="2" y="8" width="17" height="8" rx="1.5" />
    <path d="M21.5 10.5v3" />
    <rect
      x="4"
      y="10"
      width={Math.max(0.6, 13 * Math.min(1, Math.max(0, level)))}
      height="4"
      rx="0.5"
      fill="currentColor"
      stroke="none"
    />
  </svg>
);

export const IconModel = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <path d="M12 3l8 4.5v9L12 21l-8-4.5v-9L12 3z" />
    <path d="M12 12l8-4.5M12 12v9M12 12L4 7.5" />
  </svg>
);

export const IconLogs = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <path d="M4 6h10M4 12h16M4 18h7" />
    <path d="M18 5l2 2-2 2" />
  </svg>
);

export const IconMonitor = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <path d="M3 12h4l2 5 4-10 2 5h6" />
  </svg>
);

export const IconUsage = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <path d="M4 20V10M10 20V4M16 20v-7M22 20H2" />
  </svg>
);

export const IconSkills = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <path d="M12 2l2.4 5.6L20 8l-4 4 1 6-5-2.8L7 18l1-6-4-4 5.6-.4L12 2z" />
  </svg>
);

export const IconCache = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <ellipse cx="12" cy="5.5" rx="8" ry="2.8" />
    <path d="M4 5.5v6c0 1.6 3.6 2.8 8 2.8s8-1.2 8-2.8v-6" />
    <path d="M4 11.5v6c0 1.6 3.6 2.8 8 2.8s8-1.2 8-2.8v-6" />
  </svg>
);

export const IconConsole = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <rect x="3" y="4" width="18" height="16" rx="2" />
    <path d="M3 9h18M6.5 6.5h.01M9 6.5h.01" />
  </svg>
);

/** 代理（可接管客户端）：小机器人头。 */
export const IconAgents = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <rect x="4" y="8" width="16" height="11" rx="3" />
    <path d="M12 8V5M9.5 5h5" />
    <circle cx="9.5" cy="13" r="1" />
    <circle cx="14.5" cy="13" r="1" />
    <path d="M9.5 16h5" />
  </svg>
);

/** 虚拟键盘：圆角键盘 + 键点（映射页虚拟键盘面板直达入口）。 */
export const IconKeyboard = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <rect x="2.5" y="6" width="19" height="12" rx="2" />
    <path d="M6 10h.01M9.5 10h.01M13 10h.01M16.5 10h.01M8 14h8" />
  </svg>
);

/** 遥控器：圆角机身 + 方向键 + 中心确认键（与其余导航图标同族线条）。 */
export const IconRemote = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <rect x="7" y="2.5" width="10" height="19" rx="4" />
    <circle cx="12" cy="8" r="2.2" />
    <path d="M12 13v5M9.5 15.5h5" />
  </svg>
);

/** 蓝牙：标准符文折线（遥控器页扫描/连接动作）。 */
export const IconBluetooth = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <path d="M7 7.5 17 16.5 12 21V3l5 4.5L7 16.5" />
  </svg>
);

export const IconSettings = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <circle cx="12" cy="12" r="3" />
    <path d="M19 12a7 7 0 00-.1-1.2l2-1.5-2-3.4-2.3 1a7 7 0 00-2-1.2L14.2 3h-4l-.4 2.5a7 7 0 00-2 1.2l-2.3-1-2 3.4 2 1.5A7 7 0 005 12c0 .4 0 .8.1 1.2l-2 1.5 2 3.4 2.3-1a7 7 0 002 1.2l.4 2.5h4l.4-2.5a7 7 0 002-1.2l2.3 1 2-3.4-2-1.5c.1-.4.1-.8.1-1.2z" />
  </svg>
);

export const IconSun = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <circle cx="12" cy="12" r="4" />
    <path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4" />
  </svg>
);

export const IconMoon = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <path d="M21 12.8A8.5 8.5 0 1 1 11.2 3a6.8 6.8 0 0 0 9.8 9.8z" />
  </svg>
);

/** 「有流量自动弹出速率线」开关：一条速率线 + 右端向上弹出的箭头。 */
export const IconFlowAuto = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <path d="M2 14h3l2.5-6L10 16l2-5h3" />
    <path d="M19.5 15V7.5M17 10l2.5-2.5L22 10" />
  </svg>
);

/** 接单开关：电源符号 = 推理入口开/关。暂停时多一道斜杠（AGENTS：状态不得只靠颜色，
 *  形状必须跟着变），配色另用状态语义色。与其余顶栏图标同族线条。 */
export const IconAccept = ({ paused = false, ...p }: SVGProps<SVGSVGElement> & { paused?: boolean }) => (
  <svg {...base(p)}>
    <path d="M12 3v8" />
    <path d="M6.6 6.6a8 8 0 1 0 10.8 0" />
    {paused && <path d="M4 20 20 4" />}
  </svg>
);

/** 主题「跟随系统」档：半明半暗的圆（对比度符号），与日/月同一族线条。
 *  三态按钮（深绿/暖白/跟随系统）里代表 auto 那一档。 */
export const IconThemeAuto = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <circle cx="12" cy="12" r="9" />
    <path d="M12 3a9 9 0 0 1 0 18z" fill="currentColor" stroke="none" />
  </svg>
);

/** 展开用的右尖角（配合 group-open / rotate 类名使用）。 */
export const IconChevron = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <path d="M9 6l6 6-6 6" />
  </svg>
);

/** 探活（⟳）。 */
export const IconRefresh = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <path d="M20 12a8 8 0 1 1-2.34-5.66" />
    <path d="M20 4v4h-4" />
  </svg>
);

export const IconPencil = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <path d="M12 20h9" />
    <path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4Z" />
  </svg>
);

/** 格式化（魔棒 + 星芒，P3-6：原先用 emoji 当功能图标）。 */
export const IconWand = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <path d="M3 21l10-10" />
    <path d="M15 3l1.4 3.1L19.5 7.5l-3.1 1.4L15 12l-1.4-3.1L10.5 7.5l3.1-1.4L15 3Z" />
    <path d="M19.5 15.5v3" />
    <path d="M18 17h3" />
  </svg>
);

/** 槽位调整（两条滑轨 + 滑块）：fix:model-matrix-ui 3 号——矩阵格子里原先用「槽」
 *  字当功能图标（audit 图标归族口径），换成同族描边图标。 */
export const IconSlots = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <path d="M4 8h8M18 8h2" />
    <circle cx="15" cy="8" r="2.2" />
    <path d="M4 16h2M12 16h8" />
    <circle cx="9" cy="16" r="2.2" />
  </svg>
);

export const IconTrash = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <path d="M3 6h18M8 6V4h8v2M6 6l1 14h10l1-14" />
  </svg>
);

/** 锁（受保护资源：删不掉，点它只为问一句「为什么」）。 */
export const IconLock = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <rect x="4" y="10.5" width="16" height="10" rx="2" />
    <path d="M8 10.5V7a4 4 0 0 1 8 0v3.5" />
  </svg>
);

/** 悬浮窗（画中画：大窗 + 右下角小浮窗）。 */
export const IconFloat = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <rect x="3" y="5" width="18" height="14" rx="2" />
    <rect x="12" y="11.5" width="8" height="6" rx="1.5" />
  </svg>
);

/** 悬浮窗「隐藏」态（大窗 + 右下角 ✕）：顶栏开关随状态换图标，语义 = 点击隐藏。 */
export const IconFloatHide = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <rect x="3" y="5" width="18" height="14" rx="2" />
    <path d="M13.5 13l5 4M18.5 13l-5 4" />
  </svg>
);

/** 资源悬浮小窗（顶栏快捷开关）：小窗里一条脉搏线，语义 = CPU/内存监视。 */
export const IconSysFloat = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <rect x="4" y="6" width="16" height="12" rx="2" />
    <path d="M7 13l2.5-3 2.5 3 2.5-4 2.5 4" />
  </svg>
);

/** 提示词速取面板（顶栏唤起按钮）：终端窗口里一个 `>` 提示符，语义 = 唤起面板。 */
export const IconPromptPanel = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <rect x="3" y="5" width="18" height="14" rx="2" />
    <path d="M7 10l2.5 2L7 14M11.5 14.5H16" />
  </svg>
);

export const IconCheck = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <circle cx="12" cy="12" r="9" />
    <path d="M8.5 12.5l2.5 2.5 4.5-5" />
  </svg>
);

export const IconAlert = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <circle cx="12" cy="12" r="9" />
    <path d="M12 7.5v5.5M12 16.3h.01" />
  </svg>
);

export const IconInfo = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <circle cx="12" cy="12" r="9" />
    <path d="M12 11v5.5M12 7.7h.01" />
  </svg>
);

export const IconClose = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <path d="M6.5 6.5l11 11M17.5 6.5l-11 11" />
  </svg>
);

export const IconSearch = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <circle cx="11" cy="11" r="6.5" />
    <path d="M16 16l4.5 4.5" />
  </svg>
);

export const IconCopy = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <rect x="9" y="9" width="11" height="11" rx="2" />
    <path d="M15 5.5H6a1.5 1.5 0 0 0-1.5 1.5v9" />
  </svg>
);

/* ui_01 原型回灌 U2：边条品牌标记与折叠钮。与全族同一笔画（strokeWidth 1.7、圆头）。 */

export const IconBolt = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <path d="M13 2L3 14h9l-1 8 10-12h-9l1-8z" />
  </svg>
);

export const IconPanelCollapse = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <rect x="3" y="3" width="18" height="18" rx="2" />
    <path d="M9 3v18" />
    <path d="M16 9l-3 3 3 3" />
  </svg>
);

/* ui_01 原型回灌 U11：把散在页面里顶替图标的 unicode 字形收进同一族
   （同一 strokeWidth 1.7、圆头、fill none）。字形当图标有两个硬伤：读屏念不出（或念成
   「黑圆点」），且 WKWebView 不渲染 title，原先挂在旁边的解释本来就是空的。 */

/** 折叠指示：收起朝右、展开转 90° 朝下（旋向口径同 BackendRowItem）。
    页面上原先是 ▾/▸ 两个字符，五处各写一遍。 */
export const IconCaret = ({
  open = false,
  className = "",
  ...p
}: SVGProps<SVGSVGElement> & { open?: boolean }) => (
  <IconChevron width={12} height={12} {...p} className={`shrink-0 ${open ? "rotate-90" : ""} ${className}`.trim()} />
);

/** 新增一行（原先是「+ 添加模型」那个加号字符）。 */
export const IconPlus = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <path d="M12 5v14M5 12h14" />
  </svg>
);

/** 默认模型标记（矩阵格里原先是 ★ 字符）。Skills 那个星是边条导航标记，形状各管各的，不混用。 */
export const IconStar = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <path d="M12 3.6l2.6 5.3 5.8.9-4.2 4.1 1 5.7-5.2-2.7-5.2 2.7 1-5.7-4.2-4.1 5.8-.9L12 3.6z" />
  </svg>
);

/** 有任务在跑时的转圈（后台任务托盘）。底圈是轨道、只压不承载信息的装饰，压暗不许用在文字上。 */
export const IconSpinner = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <circle cx="12" cy="12" r="9" opacity="0.25" />
    <path d="M21 12a9 9 0 0 0-9-9" />
  </svg>
);

/* ui_01 原型回灌 U4：表头排序标记（未排序态的双向箭头；已排序态复用 IconChevron 转向）。 */

export const IconSort = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <path d="M8 10l4-4 4 4" />
    <path d="M8 14l4 4 4-4" />
  </svg>
);

/* 遥控器按键映射（docs/ui_02/mapping.html 原型键图标，16 viewBox 路径 ×1.5 换算到 24 族）。 */

export const IconKeyPower = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <path d="M12 3.75v7.5" />
    <path d="M6.9 6.3a7.2 7.2 0 1 0 10.2 0" />
  </svg>
);

export const IconKeyUp = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <path d="M12 19.5V4.5M6.75 9.75L12 4.5l5.25 5.25" />
  </svg>
);

export const IconKeyDown = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <path d="M12 4.5v15M6.75 14.25L12 19.5l5.25-5.25" />
  </svg>
);

export const IconKeyLeft = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <path d="M19.5 12H4.5M9.75 6.75L4.5 12l5.25 5.25" />
  </svg>
);

export const IconKeyRight = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <path d="M4.5 12h15M14.25 6.75L19.5 12l-5.25 5.25" />
  </svg>
);

export const IconKeyOk = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <circle cx="12" cy="12" r="8.1" />
    <circle cx="12" cy="12" r="2.4" />
  </svg>
);

export const IconKeyBack = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <path d="M15.75 5.25L9 12l6.75 6.75" />
  </svg>
);

export const IconKeyHome = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <path d="M4.5 11.25L12 4.5l7.5 6.75M6.75 10.5v8.25h10.5v-8.25" />
  </svg>
);

export const IconKeyMenu = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <path d="M5.25 7.5h13.5M5.25 12h13.5M5.25 16.5h13.5" />
  </svg>
);

export const IconKeyPlus = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <path d="M12 5.25v13.5M5.25 12h13.5" />
  </svg>
);

export const IconKeyMinus = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <path d="M5.25 12h13.5" />
  </svg>
);

export const IconKeyTv = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <rect x="3.9" y="6" width="16.2" height="11.4" rx="2.1" />
    <path d="M9 20.4h6" />
  </svg>
);

/** 麦克风：语音录入卡（计划 slot:S-F4）。 */
export const IconMic = (p: SVGProps<SVGSVGElement>) => (
  <svg {...base(p)}>
    <rect x="9" y="2.5" width="6" height="11" rx="3" />
    <path d="M5.5 11a6.5 6.5 0 0013 0M12 17.5V21M9 21h6" />
  </svg>
);
