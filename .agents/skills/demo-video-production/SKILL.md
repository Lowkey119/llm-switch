---
name: demo-video-production
description: 给一个软件产品录功能演示片的完整流程：一问一答收集意图 → 写每段分镜文案 → 先出配音并让用户逐段确认 → 再按配音卡点分段录制 → 用户确认后组装成片。触发：录演示视频/宣传片/功能演示片/重录演示片/给这个项目出个片子/分镜文案/口播配音/配音对不上画面/录出来没声音/成片怎么拼。适用于 Tauri/Electron/网页/终端 App 的界面演示。
---

# 软件功能演示片制作（demo-video-production）

一句话流程：**文案 → 配音 → 确认 → 录制卡点 → 确认 → 组装**。
配音先于录制，因为录制的动作全部要对齐语音句起点；反过来录必然对不上。

## When to Use

- 用户要给某个软件录一段带中文口播的功能演示片（或重录）。
- 已有素材但配音/画面错位，要重建对轴口径。
- 用户丢来「分镜」「口播」「成片」这类词，且对象是一个可运行的界面。

不适用：纯静态图/幻灯（用卡片渲染即可）、需要真人出镜或外部拍摄。

## 铁律（顺序不可颠倒）

1. **先文案后配音，先配音后录制。** 录制时每个动作都锚定「第 n 句口播的起点」，所以语音必须先存在。
2. **配音必须经用户逐段试听确认**才进录制。音色/语速一改，全部时长重算，录了就白录。
3. **每镜单独录、单独出配音片段**（`out/配音/NN.mp4`），交付确认前不要拼成片。
4. **全部分镜录完 + 用户确认，才组装**（`edit.py` → `mix.py`）。
5. 全程**一问一答**，一次只问一个问题，给 2–3 个方向 + 推荐项 + 理由；说人话，不甩术语、不甩代码。

## 目录组织（每个片子一个工程目录）

```
<日期>-<产品>-<主题>/
├── 口播文案/            # 每镜一个 md：NN.md（画面提示 + 口播正文 + 真人动作标注）
├── docs/                # 分镜总表、重录方案、验收清单
├── tools/               # 本技能落地的脚本（见下）
├── voice/               # 逐句 wav：<镜号>_<序号>.wav（删掉即重合成）
├── shots/               # 本轮录的原始素材
├── shots_prev/          # 上一批素材（重录时留底，别删）
├── shots_trim/          # 剪掉静止段的素材 + 重映射后的 cues.json
├── out/                 # intro.mp4 / 卡片 / cut.mp4 / final.mp4 / timeline.json
│   └── 配音/            # 逐镜「画面+旁白」交付片段（给用户逐段看的东西）
├── assets/              # logo、BGM、截图素材
└── logs/                # 每镜录制日志、连播日志
```

## Procedure

### 阶段 0 · 一问一答收集意图

一次一个问题，按这个顺序问（每题给方向 + 推荐）：

1. **一句话定位**：这片子要让人记住什么？（例：聚合多机算力 / 用说话摆脱打字 / 用遥控操纵代理）
2. **高潮段**：全片压轴是哪个模块？（决定排片顺序，高潮放最后）
3. **模块清单与顺序**：要覆盖哪些页面/功能，哪些明确不拍。
4. **时长预期**：不硬卡，按口播自然长度估（一般 12–18 分钟）。
5. **视觉基调**：浅色/深色（录制主题要和 App 主题一致）、鼠标点击是否要高亮动画。
6. **真人动作**：哪些步骤脚本点不了（物理按键、拖拽安装、系统弹窗）——列出来，录制时冒泡给用户。
7. **配音口径**：见阶段 2。
8. **转场口径**：见阶段 2。

把答案写进 `docs/` 的方案文档，之后所有决策引用它，别凭记忆。

### 阶段 1 · 写分镜文案

- 每镜一个 `口播文案/NN.md`：镜号、页面/页签、动作序列（含真人段）、口播正文。
- 同步写进 `tools/narr.py`：`ORDER = [...]`（镜序）+ `NARR = {'NN': '句子 / 句子 / ...'}`（` / ` 分句）。
- 文案口径：陈述事实，不玩梗；**不解释实现细节、不解释 bug、不谈未来计划**；已经在工作的功能别写成「还没做」。
- 术语在屏幕上保持原样，只在合成时替换读法（见 `HOMOPHONE`）。
- 写完跑 `tools/dump_copy.py` 导出每镜文案给用户过目。

### 阶段 2 · 交付确认前必须收集的两组口径

**配音（缺一不可，逐项问）**
- 合成通道：本地 TTS 服务地址 / 上游模型 / 别名。
- **音色**（voice=…）：给 2–3 个候选，各出一句样音让用户挑。
- **语速**：默认自然语速；用户要快/慢就记倍率，并同步改录制侧的等待时长。
- 停顿：句间 gap、镜头起口的 `SPEECH_LEAD`（默认 0.35s）、片头 BGM 前置 lead。
- 特殊读法：产品名/英文缩写的同音替换表（例：`llm-switch` → 合成时写 `LLM-Switch`，屏幕文字不动）。
- **复核**：用 ASR 把合成结果转回来对字（`tools/asr_check.py`），抓吞字/复读/降质。
- **闸门**：单句时长超过 `0.35 s/字` 直接判失败重合成（TTS 偶发拖音能到十几秒）。

**转场**
- 镜间转场类型（xfade：fade / wipeleft / slideup…）与时长（默认 0.4s）。
- 片头/片尾是否单独渲染（动画 logo、字标），BGM 音量与前置秒数。
- 卡片（无法录屏的说明段）用静态卡还是动画卡。

### 阶段 3 · 先出配音

```bash
python3 tools/tts_place.py --plan          # 合成全部句 → voice/*.wav + out/voice_plan.json
python3 tools/voice_preview.py             # 逐镜拼一条试听轨
python3 tools/asr_check.py                 # ASR 回读，抓错读
```

`voice_plan.json` 是后面一切的对轴源：每镜的 `sents / durs / starts / speech`。
**逐镜给用户听，改哪镜删哪镜的 wav 重合成。** 全部点头才进下一阶段。

### 阶段 4 · 分段录制（时长卡点）

- 对轴口径：`素材起点 + SPEECH_LEAD + voice_plan.starts[n]` = 第 n 句开口处；动作步骤用 `{"cue": n}` 或 `at: n` 对齐。
- 每镜一个新会话录（`tools/rec.sh NN`），主会话只收一行结论，日志留 `logs/rec_NN.log`。
- 录制前必查三件事：① OBS 录的是**正确那块屏**（多屏机每块屏 UUID 不同）② App 在**前台**（脚本跑在终端里，前台是终端时点击全打在终端上）③ 元素名解得到（`python3 tools/axfind.py 名字`）。
- 真人步骤用 `{"human": {"say": "...", "secs": N}}`，弹通知等人，别催；**用户不在机器前就别开这一镜**。
- 每镜录完抽 3–4 帧自检：画面是不是 App、指针动没动、YAVG 别接近 0（黑屏）。
- 出逐镜配音片段给用户看：`python3 tools/voice_clip.py` → `out/配音/NN.mp4`；连播 `python3 tools/watch.py`。

### 阶段 5 · 组装成片（用户确认所有分镜之后）

```bash
python3 tools/trim_static.py    # 剪静止段，同时重映射 cues 到剪后时间轴
python3 tools/edit.py           # shots_trim/ 按 ORDER 拼 out/cut.mp4 + timeline.json
python3 tools/tts_place.py      # 按 timeline.json 把每句摆到成片时间轴
python3 tools/mix.py            # 旁白 + BGM(+软字幕) → final.mp4
```

交付时给：`final.mp4` + 每镜 `out/配音/NN.mp4` + `docs/` 里的分镜表。

## 要落地的脚本（可从参考工程拷贝后按本项目改）

| 脚本 | 职责 |
|---|---|
| `narr.py` | `ORDER` + `NARR`（口播唯一真源，` / ` 分句） |
| `shots.py` | `MANIFEST`：每镜动作步骤（`page/tab/hover/mouse/click/clickxy/term/wait/cue/human`） |
| `shoot.py` | 执行步骤 + 开录 + cue 对齐；每步 try/except，解不到元素只跳该步 |
| `tts_place.py` | 合成 + 排布；`--plan` 出 voice_plan，正式跑按 timeline 摆句 |
| `trim_static.py` | 剪静止段并重映射 cues |
| `edit.py` | 粗剪 + `timeline.json`（xfade 时长在这里） |
| `mix.py` | 终混（旁白 + BGM + 软字幕） |
| `axfind.py` | 元素名 → 屏幕坐标（**递归走 AX 树**，别用 `entire contents`） |
| `obs_rec.py` | obs-websocket 5.x：建场景/开关录/取文件 |
| `voice_clip.py` | 逐镜「画面+旁白」片段（交付确认用） |
| `watch.py` | 逐段连播（QuickTime，支持 skip/stop 标志文件） |
| `voice_preview.py` / `asr_check.py` / `dump_copy.py` | 试听轨 / ASR 复核 / 导出文案 |
| `preflight.py` | 录制前断言（屏、场景、App、语音齐备） |

## Pitfalls

- **`entire contents` 在 webview（Tauri/Electron/WKWebView）AX 树上会返回 0**：名字明明在树里却一个取不到。自己递归走树（全树约 3s），批量版记得 `local` 声明要放在 handler 顶部、`tell` 块里不认 `local`（否则 -2741）。
- **多屏机 OBS 录错屏**：`screen: 0` 会静默录黑屏；必须 `display_uuid`，且换过显示器/升过 OBS 后 UUID 会变。用「录 3s 抽帧看有没有菜单栏」来判定，别信注释。
- **前台不是 App**：脚本在终端里跑，点击会打在终端上，画面毫无反应但脚本以为成功。每个动手步骤前 `ensure_frontmost()`。
- **Retina 截图是 2x**，`cliclick` 用 point 坐标；`cliclick m:` 只移动，`c:` 才点击。
- **TTS 缓存按 `voice/镜号_序号.wav`**：改了文案必须删 wav 才会重合成；预览文件用带时间戳的文件名（QuickTime 不重载同名覆盖的文件）。
- **素材比口播短**：交付片段用最后一帧 `tpad` 冻住补齐，别截话；但根因是录短了，补录时留尾。
- **真人段要用户在场**：不在就改期，别用「假装做了」的素材糊过去。
- **别把在工作的功能讲成待办**，也别在口播里解释实现和 bug。

## Verification

- `out/voice_plan.json` 覆盖所有镜；每句时长在 `0.05–0.35 s/字` 区间内。
- ASR 回读与文案一致（产品名、英文缩写重点看）。
- 每镜 `out/配音/NN.mp4`：`ffmpeg -i f -af volumedetect -f null -` 的 `mean_volume` 在 -20~-14dB（不是静音）。
- 每镜抽帧确认：画面是被录的 App、指针在动、`YAVG` 不接近 0。
- 录制日志里 `cue 没走到` 的清单为空（或已确认可接受）。
- 组装后 `out/timeline.json` 的 `total` 与 `final.mp4` 时长一致；用户逐段确认过才组装。
