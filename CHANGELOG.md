# 更新日志

> 每个版本的改动在此登记，CI 出包时把对应小节写进 GitHub Release 正文，
> Gitee Release 正文同步带上同一份（见 `.github/workflows/release.yml` 与 `scripts/gitee-release-sync.sh`）。
> 口径：只写用户看得见的变化（修了什么 / 优化了什么 / 新增什么），不写内部流水账。

## v0.2.105

**新增 / Added**

- **媒体请求也上实时页**：语音合成、语音转写、图片生成现在和聊天请求一样，在观测·实时与浮动面板里挂号——
  走的是哪个后端、哪个模型、在飞多久、传了多少字节都看得见。之前它们静默经网关转发，页面上不留痕迹。
  *Media requests now show up in the live monitor: speech synthesis, transcription and image generation are tracked in Observe → Live and in the floating panel just like chat traffic — backend, model, in-flight duration and bytes. Before, they passed through the gateway silently.*

## v0.2.104

**新增 / Added**

- **虚拟声卡转写全链路打通**：遥控器语音键 → 设备 🌐 键映射（修复 HID 功能码编码，
  系统认得改写后的 Fn）→ 输入法 HUD 实时转写 → 文字直接进当前焦点输入框；
  输入法可选可校验，卡头实时回显就绪状态。
  *The virtual-soundcard transcription route now works end to end: the remote's voice key maps
  to the device 🌐 key (fixed the HID usage-page encoding so macOS honors the remapped Fn),
  your input method transcribes live and the text lands in the focused field; pick and verify
  the input method, with a ready badge on the card header.*
- **托盘「遥控器」子菜单**：语音转写路快捷切换（系统转写 / 虚拟声卡 / 网关转写 / 关闭语音），
  勾标即当前生效路，与设置页同一份偏好。
  *Tray "Remote" submenu: quick-switch the voice route (system speech / virtual soundcard /
  gateway STT / off), checkmark shows the live route, same preference as the settings page.*
- **预存方案「另存为」**：切换方案后组合框收起时留显方案名，一眼看清当前用的是哪套。
  *Scheme "Save as"; after switching, the combo box keeps showing the applied scheme name.*

**修复 / Fixed**

- 对话框层级修复：删除确认、后端编辑等弹窗不再被顶栏与吸顶行穿透遮挡。
  *Dialogs (delete confirm, backend editor, etc.) are no longer pierced by the top bar or sticky rows.*
- 语音路不再误关：点已选中的路径卡不再关闭采集，关路改走显式「关闭」按钮。
  *Clicking the already-selected route card no longer turns capture off; use the explicit Off button.*
- 小米 RC001 语音键映射失效修复（功能码页编码按 Apple 官方口径重写）。
  *Xiaomi RC001 voice-key remap fixed (usage-page encoding rewritten per Apple's spec).*

**页面优化 / Polish**

- 语音卡改版：本地两路并一张卡与网关卡互斥，字段两列行式（标签左、控件右），
  控件统一行高；全站输入框 / 下拉框高度一并统一。
  *Voice panel rebuilt: two-column inline fields with uniform control heights, now app-wide.*
- 观测页用量区间选择器、后端页控制台工具条与参数中心作用域条并入顶部页签行；
  控制台面板占满剩余高度。
  *Usage range picker, console toolbar and params-center scope bar merged into the tab row;
  the console panel now fills the remaining height.*
- 设置页开关卡与网关日志卡改两列排版；各页顶部留白统一减半。
  *Settings toggle cards and gateway-log cards go two-column; page top padding halved app-wide.*
- 顶栏开关按钮开启时挂绿点徽标；品牌 logo 与页名加大加粗。
  *Top-bar toggles get a green dot when on; brand mark and page title enlarged and bolder.*

## v0.2.102

**新增 / Added**

- **遥控器页大改（三槽映射 + 虚拟键盘编辑）**：每个按键支持单击 / 双击 / 长按三种映射，
  全局冲突检测——撞了别的键的组合就地红闪拒绝；编辑改为内嵌虚拟键盘面板：点修饰键再点
  主键拼组合，分类功能芯片（窗口 / 媒体 / 应用 / 系统 / 视图缩放）点一下直接填入；
  实物图热点与键卡连线联动（已映射常显虚线、悬停高亮实线压顶）。
  *The remote page is rebuilt around three slots per button (click / double-click / long-press)
  with global conflict detection — a clashing shortcut flashes red and is refused. Editing now
  uses an embedded virtual keyboard: tap modifiers then a main key to build a shortcut, or
  click a categorized function chip (window / media / apps / system / view-zoom) to fill it in
  one go; the remote photo and key cards stay wired together.*
- **预存方案**：把当前映射存成命名方案、多套一键切换（覆盖前弹警告确认），并支持改名、
  导出 JSON 文件、从文件导入。
  *Preset schemes: save the current mapping under a name, switch sets with a confirm warning,
  and rename, export to a JSON file, or import one back.*
- **功能键区**：键列下方挂分类功能芯片——编辑态全量可点填入，运行态只显示已映射的
  （绿框只读）；正在录制的槽优先作为填入目标：点槽、点芯片，一步完成映射。
  *Function-chip area under the key columns: fully clickable while editing; in view mode only
  mapped chips remain (green, read-only). A focused recording slot is filled first — click a
  slot, click a chip, done.*
- **设备档案抽象**：小米遥控器 2 / 2 Pro 按档案识别（广播名 / 键表 / 独占策略 / 语音语义），
  为多设备扩展铺路；键位配置改为按档案的外部 JSON 文件并带缺省映射播种。
  *Device-profile abstraction: Xiaomi Remote 2 / 2 Pro are recognized by profile (advertised
  names, key tables, seize policy, voice semantics) to pave the way for multi-device support;
  key configs moved to per-profile JSON files seeded with sane defaults.*
- **边条纯图标轨**：边条收窄为 56px 纯图标轨并换上产品 logo，页名与 ⌘n 快捷键改悬停
  tooltip；版本号搬回顶栏，有更新时呼吸交替提示。
  *The rail is now a 56px icon-only track wearing the product logo; page names and ⌘n
  shortcuts live in hover tooltips. The version badge moved back to the top bar and pulses
  when an update is available.*

**优化 / Improved**

- 流量卡：模型名与逐模型指标上移卡头、曲线拉满卡宽；纯在飞的（后端，模型）也补一行，
  卡片不再等第一条请求跑完才出现。
  *Traffic cards: model name and per-model metrics moved into the card header, charts span
  the full width; in-flight-only (backend, model) pairs now get a row immediately.*
- 后端页：搜索与状态过滤并入页签行；停用后端即清健康灯并拒绝探活；停用态切换网关/直连
  先补启用，杜绝假成功。
  *Backends page: search and status filter joined the tab row; disabled backends clear their
  health light and refuse probing; switching mode while disabled re-enables first instead of
  pretending to succeed.*
- 模型行两列制：长名字自由换行，读数 / 开关钉右不再被挤到第二行；控制台后端标签卡宽
  随名字自适应、顶行滚动吸顶。
  *Model rows use a two-column layout — long names wrap while metrics and toggles stay pinned
  right; console backend tabs size to their names and stick while scrolling.*
- 本窗视图缩放让路系统辅助功能缩放：⌥⌘± 不再被应用自身吃掉（应用缩放只认裸 ⌘±/⌘0），
  遥控器可以映射 macOS 的缩放快捷键了。
  *In-app zoom no longer swallows ⌥⌘± (it only reacts to plain ⌘±/⌘0 now), so the remote can
  drive macOS's own zoom hotkeys.*

**修复 / Fixed**

- 修复吸顶表头穿透留缝：日志表头、遥控器卡头滚动时不再从固定条上方穿过。
  *Fixed sticky headers sliding through the fixed bar with a visible gap (logs table, remote
  card header).*
- 流量卡不再把同一模型裂成「在飞行 + 上游 id」两行——完成记账改按挂号别名。
  *A model no longer splits into two traffic rows ("in-flight" + upstream id); completion is
  booked under the registered alias.*
- 遥控器录制拦截病根修复（事件 tap 从未挂进主 runloop，回调从未被调用）：Esc 可录、
  录制中修饰键不再漏进系统；菜单键长按的系统 autorepeat 被吞；禁用表按真机键表纠偏
  （补返回键、解禁音量键）；mic 键并入设备级禁用表。
  *Recording capture actually works now (the event tap was never attached to the main runloop):
  Esc is recordable, modifiers no longer leak while capturing; menu-key long-press swallows
  system autorepeat; the disable table was corrected against real key reports (back key added,
  volume keys released) and the mic key joins the device-level disable table.*
- 电源键改写表看门狗：共享模式的按键改写表被抹掉（设备重连、被其他应用整表覆盖）时
  能检出并告警——否则电源键会直通关机。
  *Power-key watchdog: if the shared-mode key remap table gets wiped (device reconnect, another
  app overwriting it) it is detected and warned about — otherwise the power key would shut the
  Mac down directly.*
- 机位分派串台：HAL/GU 机位由编译期常量改运行期静态（halogen 一直在读写 gufo 的机位）。
  *Fixed machine-slot cross-talk: HAL/GU slots became runtime statics (halogen had been
  reading and writing gufo's slot all along).*
- 发布仓弹窗支持一键复制；外链窗口二次点击唤不回的问题修复。
  *Release-repos dialog supports one-click copy; clicking an external-link window twice now
  actually brings it back.*

## v0.2.76

**修复 / Fixed**

- **顶栏提示跟随界面语言**：上一版把中英两行一起显示，理解错了需求；现在切到中文只显中文、
  切到英文只显英文（自绘 tooltip 保留，因为 WKWebView 不渲染原生 `title`）。
  *Top-bar tooltips now follow the interface language. The previous release showed Chinese
  and English at once, which was not the request: with the UI in Chinese you get Chinese,
  in English you get English (the in-app tooltip stays, because WKWebView ignores the
  native `title` attribute).*

**已知问题 / Known issue**

- **每次更新后需要重新勾选辅助功能**：macOS 发行包目前是链接器自动 ad-hoc 签名（CI 未做
  Developer ID 签名），系统把每个新构建当成新 App。代理 → 提示词与命令 里的
  「清掉旧授权，重新勾选当前版本」可一键清掉旧条目再重新授权；根治需要把 Developer ID
  证书接入 CI。
  *Accessibility must be re-granted after every update: macOS artifacts are only ad-hoc
  linker-signed (no Developer ID in CI), so the system treats each build as a new app.
  Use "Reset the old grant" under Agents → Prompts & commands to clear the stale entry
  and re-check it; the permanent fix is putting a Developer ID certificate into CI.*

## v0.2.75

**修复 / Fixed**

- **速率线认错代理**：Claude Code 在跑，速率线却挂在 opencode 上。归属判定原来拿整条
  命令行撞子串，而 Orca 传给 claude 的 `--prefill` 提示词里正好写着 `opencode`；
  现在自由文本参数一律不参与判定，只认命令名与安装路径。
  *Rate lines attributed to the wrong agent: a Claude Code session showed up under
  opencode because identity was matched against the whole command line and the
  `--prefill` text happened to contain "opencode". Free-text arguments are now
  ignored; only the command name and install path decide identity.*
- **辅助功能每次更新都重新弹窗**：macOS 发行包是 ad-hoc 签名，系统认的身份就是构建指纹，
  旧条目还勾着但新构建是新身份。代理 → 提示词与命令 里新增「清掉旧授权，重新勾选当前版本」，
  一键重置本 App 自己的条目。
  *Accessibility re-prompted after every update: ad-hoc signed builds are identified by
  their code hash, so a new build is a new app to the system while the old entry stayed
  enabled. A new "Reset the old grant" action clears this app's own entry.*
- **深色主题下选中行看不见**：列表高亮用的是半透明灰，落在墨绿底上几乎没有差别；
  改为主题化的选中底色加描边，浅色深色都一眼可辨。
  *The selected row was invisible in the dark theme — a translucent grey highlight on a
  dark green background. Selection now uses theme tokens for fill plus an outline, in
  both light and dark.*
- **模型分配只能点一次**：某模型已经是该代理的当前模型时，再点那盏灯只提示"正在使用"，
  发现分错了却无处可改；现在原位弹出「设为当前 / 移出清单」，可重复改分配并支持取消。
  *Assigning a model was one-shot: when it was already the agent's current model,
  clicking the chip only said "in use", so a wrong choice could not be undone in place.
  The chip now opens "set as current / remove from list" so re-assignment and
  cancellation both happen where you made the mistake.*

**优化 / Improved**

- **提示词面板键盘不再挑焦点**：搜索框里方向键与回车随时生效——上下移选择、回车注入；
  左右键在光标到头/尾或搜索框为空时切换 tab，中间态仍归文字光标。
  *The prompt palette now takes keys regardless of focus: Up/Down move the selection and
  Enter injects even from the search field; Left/Right switch tabs at the caret bounds or
  when the field is empty, and still move the text cursor in between.*
- **提示词面板改版**：代理标签移到左侧竖排（代理多起来不再把列表挤没）；多行编辑器移到
  列表右侧并自动加宽窗口；去掉搜索框右侧的关闭按钮（收起用 Esc 或顶栏开关）。
  *Prompt palette redesign: agent tabs moved to a left rail, the multi-line editor moved
  to a right-hand pane with the window widening for it, and the close button next to the
  search field is gone (Esc or the top-bar toggle collapses the panel).*
- **顶栏十处双语提示**：状态胶囊、今日 token、六个快捷按钮、版本号与语言键都有
  中英双语 tooltip（此前 WKWebView 不渲染原生 title，改为自绘）。
  *Ten bilingual tooltips across the top bar (status pill, today tokens, six quick
  toggles, version chip, language key) — drawn in-app because WKWebView ignores the
  native `title` attribute.*
- **实时监控新增「代理 · 进程」列**：在飞请求直接显示发起方代理与 pid，归属对不对
  拿 `ps` 一比对就知道，不用再猜。
  *Monitor now shows the client app and pid per in-flight request, so attribution can be
  verified against `ps` instead of guessed.*

**新增 / Added**

- **「项目接管」tab**：提示词面板新增技能清单，选中即把技能名注入到目标输入框，
  同时在右侧只读区展开该技能的文档；技能内容不可在此编辑。
  *New "Skills" tab in the prompt palette: selecting a skill injects its name into the
  target app and shows its documentation in a read-only right-hand pane; skill content
  cannot be edited here.*
- **发布说明双语化**：CHANGELOG 恢复逐版登记并中英双语，GitHub 与 Gitee 的 Release
  正文由同一份小节生成。
  *Release notes are bilingual again: CHANGELOG is maintained per version in Chinese and
  English, and both GitHub and Gitee release bodies are generated from that one section.*

## v0.2.74

- **顶栏改版**：状态胶囊整行居中，双击胶囊切换接单（原独立开关按钮移除）；
  端口编辑仍在胶囊内，点击不会误触切换。
  *Top bar: status pill centered, double-click toggles accepting requests; port editing
  stays inside the pill without triggering the toggle.*
- **改端口联动重写接管配置**：换暴露端口后，已接管的各家客户端配置自动改到新端口。
  *Changing the exposed port now rewrites taken-over client configs to the new port.*
- **提示词与命令管理页**：代理页新增「提示词与命令」，按 tab 增删改条目与注入方式；
  预置常用语与各代理命令库（90 条）。
  *New "Prompts & commands" view under Agents with per-tab CRUD, plus 90 seeded
  phrases and agent commands.*
- **发布页链接改用系统浏览器打开**。
  *Release links now open in the system browser.*

## v0.2.73

- **修提示词面板回车带走整个应用**：注入路径跑在非主线程上调用 CoreGraphics，
  libdispatch 断言直接 SIGTRAP 终止进程；现在激活目标 App、模拟按键全部搬回主线程，
  粘贴前先确认真前台，失败时保留剪贴板并提示手动 ⌘V。
  *Pressing Enter in the prompt panel killed the app: injection called CoreGraphics off
  the main thread, tripping a libdispatch assertion (SIGTRAP). Activation and key
  synthesis now run on the main thread, focus is confirmed before pasting, and failures
  keep the clipboard with a manual ⌘V hint.*

## v0.2.62

- **Gitee Release 不再镜像 AppImage**：AppImage 自带整套 Linux 运行时（84MB），每版重复跨境传输
  没有意义；Gitee 页保留该条目与 SHA256，下载改指向 GitHub，其余安装包照旧 Gitee 本页直下。

## v0.2.61

- **修 Windows / Linux 编译失败**：本机显卡采集的非 mac 分支里一个裸 `None` 缺类型标注，
  在 Windows / Linux 才编译得到的代码上报类型推断错误；本地门禁在 mac 上编不到它，
  到 CI 才炸（与 0.2.58 同一类问题，已补类型标注）。
- **修安装包收集空手而归**：CI 收集产物的命令把路径追加在 `find -exec` 之后，
  find 解析报错又被静默吞掉，包明明打出来了却一个都收不进，四个平台全红在
  「没找到任何安装包产物」；改为单条 `find -path '*/bundle/*'`（macOS / Linux 通用）。

## v0.2.60

- **Gitee 直接下载安装包**：Gitee Release 此前只写 GitHub 直链（包都在 GitHub，国内下载慢）；
  现在安装包与 SHA256 校验文件直接上传为 Gitee Release 附件，发版本页直下、不经 GitHub，
  正文物料表的「下载」同步改为 gitee 直链，GitHub Release 保留作备份镜像。
- **发版同步不再静默漏发**：CI 把包同步到 Gitee 的步骤失败时自动退避重试
  （约 80 分钟、断点续传），网络抖动自愈；确属故障则立即报红，不再悄悄丢 Release。
- 发版链路默认三仓直推（私人仓保底、两公开仓同树同 tag），GitHub 上行偶发抽风时
  有应急通道由 GitHub 从 Gitee 反向补齐，发版不再被网络问题卡死。

## v0.2.59

- **修发布产物收集范围**：安装包检索原先递归整个 `target/`，把 cargo 编出来的构建脚本与
  测试可执行文件也当成安装包上传了（补跑 0.2.48 时一次挂了 69 个资产，58 个是垃圾）。
  现在只认 `bundle` 目录里的产物。
- **修 Gitee 同步被整批跳过**：某个平台编译失败会让整个 build 判失败，
  连带 `sync-gitee` 被 skip，Gitee 上一个 Release 都没同步上。
  现在只要不是人为取消就照常同步——GitHub 上有几个包就写几个。

## v0.2.58

- **修 Windows / Linux 编译失败**：上一版新增的独立显卡采集里把 `json` 当函数用
  （`json` 是宏），而那段代码在非 mac 平台才编译——本地门禁在 mac 上根本编不到它，
  于是全绿过关、到 CI 的 Windows / Linux 才炸。映射逻辑已挪到本地也编译的函数里并补了单测。
- **修 CI 校验和步骤**：校验文件名直接用了带空格与括号的平台名，bash 把 `(Apple Silicon)`
  当语法错误，macOS 两个 job 全挂在这一步；现在文件名转成安全 slug，
  并按 Linux / macOS / Windows 各自可用的工具取 SHA256。

## v0.2.57

- **修复发布链路**：CI 的产物收集路径写错（本仓是 Cargo workspace，安装包落在仓库根 `target/`，
  脚本却只找 `src-tauri/target/`），导致 0.2.48~0.2.52 的 Release 只有源码包没有安装包；
  现在两处都找，且**没找到安装包就直接失败**，不再静默发空 Release。
- **修复发布链路**：CI 构建漏了 `--features gui`（该特性默认关闭），打出来的是无头二进制而非桌面壳；
  现在与本地出包口径一致。
- **修复发布链路**：macOS Intel 包不再依赖已下线的 Intel 托管 runner（旧 job 会永远排队拖死整个发布），
  改在 Apple Silicon runner 上交叉编译，包名不变。
- 每个版本的发布说明（本文件对应小节）自动写进 GitHub / Gitee 的 Release 正文。

## v0.2.56

- **新增应用内更新**：顶栏「有更新」原先是常显占位（无论有没有新版都挂着），现在接真检测——
  GitHub 为主、Gitee 为备，比 semver；**没有新版本时这个标签根本不显示**。
- 点标签打开更新面板：新版本号、发布说明、按当前平台标出该装的那个包，
  每个包给 **GitHub / Gitee 双源**直链与 SHA256。
- 下载 → 校验 SHA256 → 自动拉起系统安装器（macOS 打开 dmg、Windows 跑安装程序、Linux xdg-open）。
  **缺校验和或校验不符的包一律删除并报错，绝不拉起没校验过的包。**
- 有更新时顶栏标签在「有更新」与新版本号之间缓慢交替淡入淡出；
  系统开启「减少动态效果」时不呼吸，固定显示新版本号。
- 断网 / 限流时静默保持原状，不会误报「无更新」；手动点「重新检查」才说明失败原因。

## v0.2.55

- **「系统与引擎」本机卡按平台分支采集**：原先整条链路是 macOS 专属，
  Windows / Linux + 独立显卡用户大面积显示「—」。现在非 mac 走 `nvidia-smi`
  一次取型号 / 占用 / 显存 / 温度；拿不到的照实显示 `—`，绝不编数。
- **修正显存口径**：`vram_shared` 原先硬编码为 true，独显机器说法是错的；
  现在只有 Apple Silicon 统一内存才叫「显存与内存共享」，独显机器显示显存已用 / 总量。
- 取不到 GPU 名时标题退回平台 + 架构，不再只写光秃秃的「本机」。

## v0.2.54

- **后端控制台打不开时的说明页重排**：原先内容挤在居中 640px 小块里、正文 13px 看不清，
  周围大片空白。现在随容器铺满（上限 1080px），字号整体抬到可读区间，卡片留白同步放大。

## v0.2.53

- **halogen / gufo 版本卡去冗余**：同一件事原先说三遍（「线上最新」、
  「发现新版 A → B」徽标、下拉框「（最新）」后缀）。现在「线上最新」收窄成「线上」让位给徽标，
  下拉框的「（最新）」后缀删除；「（本地）」保留（那是本地镜像标记，不同信息）。

## v0.2.52

- **Gitee Release 自动同步**：法条要求 CI 出包后同步 Gitee Release，但工作流里从未实现，
  Gitee 那边一直要手动建。现在自动写入各包 GitHub 直链 + SHA256 + 平台对照表（Gitee 附件接口不开放给 API）。

## v0.2.51

- **接单开关挪到顶栏**，随时能点（原先只在设置页里）；暂停接单时图标加一道斜杠，
  状态不只靠颜色。与设置页、菜单栏是同一个开关，不另起状态。
- 顶栏「后台任务」按钮的提示改成报数：N 个在跑 / N 条未读 / 暂无。
- 顶栏端口号的提示补上后果：改完网关会重启，在飞请求会断。

## v0.2.50

- **安装引导改一行两列**：八家代理一家一行要滚半屏、右边还空着大半。
- 删掉读不懂的「有配置 · 未找到命令」状态徽标（八家全挂着它，等于没说；
  配置在不在下面「配置文件（N）」那一栏本来就看得见）。

## v0.2.49

- **修 pi 代理详情「当前模型（未设）」**：主 / 小模型都配着，上面那行却永远显示未设——
  那行只读默认档，而 pi 的主槽走自己的通道。现在按通道取主位真值，两处都没设显示 `—`。

## v0.2.48

- **档位槽满了不再拒绝分配**：点格子照样弹出槽位菜单，选已占的槽就是顶替。
  原先满了就只弹一句「档位槽已满」什么都不给做，是错的语义——槽位上限是「同时占用的槽数」，
  不是「满了就锁死」。
- 修矩阵格子里长槽位标签（如 `opus/sonnet/subagent`）溢出压住相邻列。

## v0.2.47

- **设置页两张「预设配置」卡合并成一张「预设与归档」**：原先两个按钮都叫「还原」，
  还原的却是不同的东西（按代理存档 vs 整包归档 / 内置预设），点哪个都不知道会覆盖回什么。
  合并后三个动作来源全写在按钮上：备份当前配置 / 整包还原（自己的备份｜内置预设）/ 按代理全部还原，
  快照落点也分别标明。
