#!/usr/bin/env python3
"""llm-switch-rate — 各代理共用的速率状态线（llm-switch 速率口径 · 通用版）
--------------------------------------------------------------------------
两级取数，同一行展示：

  1. **走 llm-switch**（优先）：速率一律取 llm-switch 网关的出口
     `GET {base}/admin/rates/line?app=<app>`（`gateway/rates.rs::line_json`），与浮窗
     单行速率线同源同口径：decode = 引擎原生速率（halogen `/metrics` 计数器差分）优先，
     原生没报或恒 0 时退回网关校准值（响应字节 × 纠偏系数）。
     归属由网关从 socket 反查（`gateway/origin.rs`），`app` 就是过滤键。
     这种模式**一个字都不算速率**，只取数与排版。

  2. **直连在线 API / 网关没跑 / 网关查不到本代理**：退回**本地估算**——读代理自己的
     会话落盘（API 进来的流量都记在那）。transcript 只有消息级时刻（内容流完才落盘），
     流式期间量不到 decode：每条消息收尾算一次端到端均值（含 prefill、偏低，带 ~），
     存进状态当「历史」。支持：claude / codebuddy（transcript JSONL）、codex（rollout
     JSONL）。gemini / copilot 的落盘没有可用计时数据 → 估不出就留空，不编数。

只显示 decode（流停了在最后一行前加「history:」前缀）：prefill 与上下文长度各代理自己显示。

三种用法（同一份文件，装进各家配置目录）：
  llm-switch-rate.py --app codex              # 单行输出，可嵌 tmux / zsh prompt
                                            # （取不到数时什么都不印——空输出就是「没数可说」）
  llm-switch-rate.py --app codex --watch      # 原地每秒刷新，放独立小窗
  llm-switch-rate.py --app claude --statusline # 状态线钩子：stdin 收 JSON，stdout 出一行
                                            # （Claude Code / CodeBuddy 的 statusLine.command）

端点发现（顺序取第一个命中的）：
  1. 环境变量 `LLMSWITCH_RATE_URL` + `LLMSWITCH_RATE_TOKEN`
  2. `<LLM_SWITCH_DATA_DIR|~/.llm-switch>/rate-ep.json` —— llm-switch 启动与改端口时
     写的发现文件 `{"base": "http://127.0.0.1:8787", "token": "..."}`

环境变量：
  LLMSWITCH_RATE_APP   代理名（等价 --app）
  LLMSWITCH_RATE_MS    --watch 刷新间隔毫秒（默认 1000）
  LLMSWITCH_RATE_TO    取数超时秒（默认 1.5，状态线每秒跑一次，不能拖）
  LLMSWITCH_NO_COLOR=1 关闭 ANSI 颜色
"""

import glob
import json
import os
import subprocess
import sys
import tempfile
import urllib.error
import urllib.parse
import urllib.request
from datetime import datetime

DIM = "\x1b[2m"
BOLD = "\x1b[1m"
YELLOW = "\x1b[33m"
CYAN = "\x1b[36m"
GREEN = "\x1b[32m"
RESET = "\x1b[0m"

PHASE_RANK = {"generating": 0, "prefill": 1, "queued": 2}


def paint(s, *codes):
    if os.environ.get("LLMSWITCH_NO_COLOR") == "1" or not codes:
        return s
    return "".join(codes) + s + RESET


def tps(n):
    """tok/s 展示：≥100 取整，否则一位小数；None → —（与浮窗 rate() 同口径）。"""
    if n is None or not isinstance(n, (int, float)):
        return "—"
    return str(round(n)) if n >= 100 else f"{n:.1f}"


def endpoint():
    """网关地址与令牌：环境变量优先，其次发现文件。→ (base, token) | None。"""
    base = os.environ.get("LLMSWITCH_RATE_URL", "").rstrip("/")
    if base:
        return base, os.environ.get("LLMSWITCH_RATE_TOKEN", "")
    data_dir = os.environ.get("LLM_SWITCH_DATA_DIR") or os.path.join(
        os.path.expanduser("~"), ".llm-switch")
    try:
        with open(os.path.join(data_dir, "rate-ep.json"), "r", encoding="utf-8") as f:
            j = json.load(f)
    except (OSError, ValueError):
        return None
    base = str(j.get("base") or "").rstrip("/")
    return (base, str(j.get("token") or "")) if base else None


def pid_chain(limit=6):
    """自身进程祖先链（自己 → 父 → …，最多 limit 层）。

    多实例分流（BUG-1）：同一家代理开多个实例时 `app` 分不出谁是谁，网关按这条
    链过滤在飞流——行的发起 pid 在链上 = 本实例（或它的子进程）发的。状态线脚本
    是被代理的 shell/hook 拉起来的，父链里才有代理进程，所以传整条而不是只传自己。
    读不到就返回空串，网关退回纯 app 过滤（旧行为）。
    """
    pids, pid = [os.getpid()], os.getpid()
    for _ in range(limit):
        try:
            with open(f"/proc/{pid}/stat", "r") as f:  # Linux
                pid = int(f.read().rsplit(")", 1)[1].split()[1])
        except (OSError, ValueError, IndexError):
            try:  # macOS/BSD: ps -o ppid= -p <pid>
                out = subprocess.run(["ps", "-o", "ppid=", "-p", str(pid)],
                                     capture_output=True, text=True, timeout=0.2).stdout
                ppid = int(out.strip())
            except (OSError, ValueError, subprocess.TimeoutExpired):
                break
            pid = ppid
        if pid <= 1:
            break
        pids.append(pid)
    return ",".join(str(p) for p in pids)


def fetch(app):
    """取本代理的速率行。→ dict | None（网关没跑/超时/格式不对都算 None）。"""
    ep = endpoint()
    if not ep:
        return None
    base, token = ep
    url = f"{base}/admin/rates/line?app={urllib.parse.quote(app)}"
    chain = pid_chain()
    if chain:
        url += f"&pids={chain}"
    req = urllib.request.Request(url)
    if token:
        req.add_header("Authorization", f"Bearer {token}")
    try:
        with urllib.request.urlopen(req, timeout=float(os.environ.get("LLMSWITCH_RATE_TO", "1.5"))) as r:
            return json.loads(r.read().decode("utf-8", "replace"))
    except (urllib.error.URLError, OSError, ValueError):
        return None


def line_row(r, many=False):
    """一条在飞交换 → 一行文案（只显示 decode，prefill 与上下文各代理自己显示）。

    `many` = 同一家此刻不止一条在飞：行首挂 `#N` 序号（网关按挂号先后给的）。
    网关分不出主代理与子代理（同进程、同入口、同模型），只敢说谁先开始——
    序号是用来把并发的几行分开的，不表示主次。
    """
    model = r.get("model") or "未知模型"
    # 网关出口就是计算值（原生或字节校准），全文照抄不加 ~：~ 只留给本地估算。
    head = paint(f"▶ {tps(r.get('decode_tps'))} tok/s", BOLD, GREEN)
    head_txt = f"{paint('#%d' % (r.get('seq') or 1), DIM)} " if many else ""
    return f"{head_txt}{paint(model, CYAN)}  {head}"


def live_rows(snap):
    """网关行里能显示 decode 的那些（queued/prefill 还没出字，没 decode 可说）。"""
    rows = (snap or {}).get("rows") or []
    return sorted((r for r in rows if r.get("phase") not in ("queued", "prefill")),
                  key=lambda r: PHASE_RANK.get(r.get("phase"), 9))


# ── 本地估算（直连在线 API / 网关没跑 / 网关查不到本代理）─────────────────────
# 数据源 = 代理自己的会话落盘。transcript 只有消息级时刻（内容在流完才落盘），
# 流式期间量不到 decode——每条消息收尾时算一次端到端均值（含 prefill、偏低，带 ~），
# 存进状态当「历史」；流停了由渲染层在行首加「history:」前缀。状态跨进程存 /tmp
# （statusline 每秒一个新进程，不存就每秒从零开始）。支持：claude / codebuddy /
# codex。gemini / copilot 的落盘没有可用计时数据 → 估不出就留空，不编数。


def _ts(s):
    """ISO8601（带 Z）→ epoch 秒；解析不了 → 0。"""
    try:
        return datetime.fromisoformat(str(s).replace("Z", "+00:00")).timestamp()
    except (ValueError, TypeError):
        return 0.0


def state_path(app):
    return os.path.join(tempfile.gettempdir(), f"llm-switch-rate.{app}.state.json")


def load_state(app):
    try:
        with open(state_path(app), "r", encoding="utf-8") as f:
            return json.load(f)
    except (OSError, ValueError):
        return {}


def save_state(app, st):
    try:
        with open(state_path(app), "w", encoding="utf-8") as f:
            json.dump(st, f)
    except OSError:
        pass  # 状态存不了就每次从零估，只是历史会丢，不影响正确性


def remember(app, text):
    """把一行速率存成「历史」（合并写，别把 transcript 偏移量冲掉）。"""
    st = load_state(app)
    st["last_text"] = text
    save_state(app, st)


def history(app):
    """最近一次速率行（网关口径或本地口径都算），没有 → ""。"""
    return str(load_state(app).get("last_text") or "")


def newest_transcript(app):
    """本代理最新的会话落盘路径；没有/不支持 → None。"""
    if app in ("claude", "codebuddy"):
        root = os.path.join(os.path.expanduser("~"),
                            ".claude" if app == "claude" else ".codebuddy", "projects")
        pat = os.path.join(root, "*", "*.jsonl")
    elif app == "codex":
        pat = os.path.expanduser("~/.codex/sessions/*/*/*/rollout-*.jsonl")
    else:
        return None  # gemini / copilot 的落盘没有可用计时数据
    best, bt = None, 0.0
    for f in glob.glob(pat):
        try:
            m = os.path.getmtime(f)
        except OSError:
            continue
        if m > bt:
            best, bt = f, m
    return best


def tail_lines(path, offset):
    """从 offset 读到文件尾的**完整行**（半行留给下次）。→ (lines, new_offset)。"""
    try:
        size = os.path.getsize(path)
        if offset > size:
            offset = 0  # 文件被截断/轮转：从头再来
        with open(path, "rb") as f:
            f.seek(offset)
            data = f.read()
    except OSError:
        return [], offset
    if not data:
        return [], offset
    cut = data.rfind(b"\n")
    if cut < 0:
        return [], offset
    lines = data[:cut].decode("utf-8", "replace").split("\n")
    return [ln for ln in lines if ln.strip()], offset + cut + 1


def est_claude(app, transcript):
    """claude / codebuddy 的 transcript JSONL → 每条消息收尾记一次 decode 历史。

    每条 assistant 消息落两条记录：内容条（含正文、output_tokens=0）与 usage 条
    （含真实 tokens）。用户条的时刻当请求起点；端到端均值 = output_tokens ÷
    （内容条 − 用户条），含 prefill、天然偏低，带 ~ 如实标注。
    """
    st = load_state(app)
    if st.get("path") != transcript:
        st = {"path": transcript, "offset": 0, "user_ts": 0, "cur": {},
              "last_text": st.get("last_text") or ""}
    lines, st["offset"] = tail_lines(transcript, st.get("offset") or 0)
    for ln in lines:
        try:
            j = json.loads(ln)
        except ValueError:
            continue
        t = j.get("type")
        ts = _ts(j.get("timestamp"))
        if t == "user":
            st["user_ts"] = ts
            st["cur"] = {}
        elif t == "assistant":
            m = j.get("message") or {}
            mid = m.get("id") or ""
            cur = st.get("cur") or {}
            if cur.get("id") != mid:
                cur = {"id": mid, "first": ts}
                st["cur"] = cur
            u = m.get("usage") or {}
            gen = u.get("output_tokens") or 0
            if gen and cur.get("first"):
                start = st.get("user_ts") or cur["first"]
                span = max(0.5, cur["first"] - start)
                st["last_text"] = f"▶ ~{tps(gen / span)} tok/s"
    save_state(app, st)


def est_codex(transcript):
    """codex rollout JSONL → 每条消息收尾记一次 decode 历史。

    response_item（reasoning/message）是内容块；token_count 的 last_token_usage
    是这条的真实 tokens。decode = 生成 tokens ÷（token_count − 首块）。
    """
    st = load_state("codex")
    if st.get("path") != transcript:
        st = {"path": transcript, "offset": 0, "first_ts": 0,
              "last_text": st.get("last_text") or ""}
    lines, st["offset"] = tail_lines(transcript, st.get("offset") or 0)
    for ln in lines:
        try:
            j = json.loads(ln)
        except ValueError:
            continue
        ts = _ts(j.get("timestamp"))
        p = j.get("payload") or {}
        pt = p.get("type")
        if pt in ("reasoning", "message") and j.get("type") == "response_item":
            if not st.get("first_ts"):
                st["first_ts"] = ts
        elif pt == "token_count":
            last = ((p.get("info") or {}).get("last_token_usage")
                    or (p.get("info") or {}).get("total_token_usage") or {})
            gen = (last.get("output_tokens") or 0) + (last.get("reasoning_output_tokens") or 0)
            first = st.get("first_ts") or ts
            if gen:
                st["last_text"] = f"▶ ~{tps(gen / max(0.5, ts - first))} tok/s"
            st["first_ts"] = 0
    save_state("codex", st)


def local_update(app, payload=None):
    """刷新本代理的本地历史（读 transcript 记账）。估不出就不动旧值。"""
    transcript = None
    if payload:
        transcript = payload.get("transcript_path") or None
    if not transcript:
        transcript = newest_transcript(app)
    if not transcript or not os.path.isfile(transcript):
        return
    try:
        if app in ("claude", "codebuddy"):
            est_claude(app, transcript)
        elif app == "codex":
            est_codex(transcript)
    except OSError:
        pass


def render(app, snap):
    """一次快照 → 要打印的行（多条在飞各占一行，解码中的在最上）。

    网关有 decode 行照抄；没有行落「历史」（transcript 收尾值，行首加「history:」）；
    都没有 → 空闲。取不到数（网关没跑 / 发现文件缺失）时**什么都不印**：这些文案
    每秒重画一遍，"llm-switch 未运行" 那种告警句只会变成噪音。
    """
    if snap is None:
        return []
    rows = live_rows(snap)
    if rows:
        r = rows[0]
        remember(app, f"▶ {tps(r.get('decode_tps'))} tok/s")
        return [line_row(r, many=len(rows) > 1) for r in rows]
    local_update(app)
    h = history(app)
    if h:
        return [f"{paint(app, CYAN)}  history: {h}"]
    return [f"{paint(app, CYAN)}  {paint('空闲', DIM)}"]


def statusline(app):
    """statusLine 钩子（Claude Code / CodeBuddy）：stdin 收 JSON，stdout 出一行。

    网关有 decode 行照抄；没行落「历史」（transcript 收尾值，行首加「history:」）；
    都没有就只报模型名——状态栏每秒重画，绝不拿别处口径的数冒充、也不常驻告警。
    """
    try:
        payload = json.load(sys.stdin)
    except Exception:
        payload = {}
    model = ((payload.get("model") or {}).get("display_name")) or app
    snap = fetch(app)
    parts = [paint(model, CYAN)]
    rows = live_rows(snap)
    if rows:
        r = rows[0]
        if len(rows) > 1:
            # 状态线只有一行，挂序号是告诉用户「此刻还有别的在飞，这是第 N 条」
            parts.append(paint(f"#{r.get('seq') or 1}", DIM))
        line = f"▶ {tps(r.get('decode_tps'))} tok/s"
        parts.append(paint(line, BOLD, YELLOW))
        remember(app, line)
    else:
        local_update(app, payload)
        h = history(app)
        if h:
            parts.append(f"history: {h}")
        elif snap is not None:
            parts.append(paint("⚡ 空闲", DIM))
    cost = (payload.get("cost") or {}).get("total_cost_usd")
    if isinstance(cost, (int, float)) and cost > 0:
        parts.append(paint(f"${cost:.2f}", DIM))
    print("  |  ".join(parts))


def main(argv):
    app, mode = os.environ.get("LLMSWITCH_RATE_APP", "cli"), "once"
    args = list(argv)
    i = 0
    while i < len(args):
        a = args[i]
        if a == "--app" and i + 1 < len(args):
            app, i = args[i + 1], i + 2
            continue
        if a in ("--watch", "--statusline", "--once"):
            mode = a[2:]
        i += 1
    if mode == "statusline":
        return statusline(app)
    if mode == "watch":
        ms = float(os.environ.get("LLMSWITCH_RATE_MS", "1000"))
        try:
            while True:
                lines = render(app, fetch(app))
                stamp = datetime.now().strftime("%H:%M:%S")
                sys.stdout.write("\x1b[H\x1b[2J" + f"{paint(stamp, DIM)}\n"
                                 + "\n".join(lines) + "\n")
                sys.stdout.flush()
                time.sleep(ms / 1000.0)
        except KeyboardInterrupt:
            return 0
    lines = render(app, fetch(app))
    if lines:
        print("\n".join(lines))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
