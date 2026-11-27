#!/usr/bin/env bash
# gitee-release-sync.sh · 把 GitHub Release 的产物同步成 Gitee Release（附件 + 正文）
#
# 口径（docs/多仓发版规范.md §五，2026-11-17 实测修订）：
#   旧结论「Gitee 附件上传接口不开放」是测错了端点（当时试的 releases/{tag}/attach 与
#   releases/attach）。实测可用的端点（10MB 真包 / .exe / 85MB 均 201，匿名直链 SHA256 一致）：
#     POST   /repos/{o}/{r}/releases/{id}/attach_files           上传（multipart：access_token + file）
#     GET    /repos/{o}/{r}/releases/{id}/attach_files           附件清单（幂等续传用；注意不是 attachments）
#     DELETE /repos/{o}/{r}/releases/{id}/attach_files/{附件id}  删附件（大小不符时重传用）
#   所以正文物料表的「下载」指向本页附件的 gitee 直链（国人不经 GitHub），GitHub 只留备份镜像一条。
#
# 幂等：同 tag 已存在 Release 就更新正文；附件按「名字 + 大小」比对，只补缺失或损坏的，
#       上层重试重复调用是安全的。
# 退出码（release.yml 的重试循环按此分类）：
#   2 = 永久错误（缺 token、GitHub 无此 Release 或零产物、Gitee 4xx）→ 立即失败，不值得重试
#   1 = 瞬态错误（网络、超时、5xx/429、回读不符）→ 交给上层退避重试
#
# 用法：
#   scripts/gitee-release-sync.sh v0.2.60                    # 本地手跑（钥匙串取凭证）
#   GITEE_TOKEN=xxx scripts/gitee-release-sync.sh v0.2.60    # CI（secrets.GITEE_TOKEN）
#
# 脚本不含密钥：Gitee token 走环境变量或 macOS 钥匙串；GitHub 产物是公开仓 API，匿名可读。
set -uo pipefail

TAG="${1:-}"
[ -n "$TAG" ] || { echo "用法: $0 <tag>（如 v0.2.60）" >&2; exit 2; }

if [ -z "${GITEE_TOKEN:-}" ] && command -v security >/dev/null 2>&1; then
  GITEE_TOKEN="$(security find-internet-password -s gitee.com -w 2>/dev/null)"
fi
[ -n "${GITEE_TOKEN:-}" ] || { echo "失败：没有 Gitee token（设 GITEE_TOKEN 或钥匙串存 gitee.com 凭证）" >&2; exit 2; }

# 更新日志与 GitHub Release 正文同源：都走 scripts/release-notes.sh 取 CHANGELOG.md
# 对应小节，两处各写一遍解析迟早分叉。取不到不致命（补跑 CHANGELOG 建立前的老 tag）。
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
NOTES="$(bash "$ROOT/scripts/release-notes.sh" "$TAG" "$ROOT/CHANGELOG.md" 2>/dev/null || true)"
export NOTES
export GITEE_TOKEN TAG
export GH_OWNER="${GH_OWNER:-Lowkey119}" GH_REPO="${GH_REPO:-llm-switch}"
export GITEE_OWNER="${GITEE_OWNER:-lowkey110}" GITEE_REPO="${GITEE_REPO:-llm-switch}"

python3 - <<'PY'
import hashlib, json, os, sys, urllib.error, urllib.parse, urllib.request, uuid

tag = os.environ["TAG"]
ver = tag[1:] if tag.startswith("v") else tag
tok = os.environ["GITEE_TOKEN"]
gh = f"{os.environ['GH_OWNER']}/{os.environ['GH_REPO']}"
ge = f"{os.environ['GITEE_OWNER']}/{os.environ['GITEE_REPO']}"


def say(m):
    print("==> " + m, flush=True)


def die(m, code=1):
    print("失败：" + str(m).replace(tok, "***"), flush=True)
    sys.exit(code)


def classify(e, ctx):
    """HTTP 错误分类：Gitee/网络的 4xx（429 除外）是永久错（端点、凭证、参数），其余是瞬态。"""
    if isinstance(e, urllib.error.HTTPError):
        if e.code == 429 or e.code >= 500:
            die(f"{ctx}：HTTP {e.code}（瞬态，可重试）", 1)
        if 400 <= e.code < 500:
            die(f"{ctx}：HTTP {e.code}（端点/凭证/参数问题，重试无用）：{e.read().decode()[:300]}", 2)
        die(f"{ctx}：HTTP {e.code}：{e.read().decode()[:300]}", 1)
    die(f"{ctx}：网络瞬态错误 {e}", 1)


def get(url, headers=None):
    req = urllib.request.Request(url, headers=headers or {})
    try:
        with urllib.request.urlopen(req, timeout=60) as r:
            return json.loads(r.read().decode())
    except (urllib.error.HTTPError, urllib.error.URLError, TimeoutError, OSError, json.JSONDecodeError) as e:
        classify(e, f"GET {url.split('?')[0]}")


# 平台对照：文件名 → 人话平台名（产物清单见 .github/workflows/release.yml 的矩阵）
def platform_of(n):
    for pat, name in [
        ("_aarch64.dmg", "macOS 11+（Apple Silicon）"),
        ("_x64.dmg", "macOS 11+（Intel）"),
        (".msi", "Windows 10+（安装包）"),
        (".exe", "Windows 10+（安装程序）"),
        (".deb", "Debian / Ubuntu"),
        (".AppImage", "Linux（AppImage）"),
        (".rpm", "Linux（RPM）"),
    ]:
        if n.endswith(pat):
            return name
    return "—"


# ---------- 1. GitHub 侧产物 ----------
say(f"读 GitHub Release {tag} 的产物")
try:
    rel = get(f"https://api.github.com/repos/{gh}/releases/tags/{tag}",
              {"Accept": "application/vnd.github+json"})
except SystemExit:
    raise
if rel is None or "id" not in rel:
    die(f"GitHub 还没有 {tag} 的 Release（CI 还没出包？message={rel.get('message') if isinstance(rel, dict) else rel}）", 2)
all_assets = rel.get("assets") or []
if not all_assets:
    die(f"{tag} 没有可同步的产物（CI 构建失败？message={rel.get('message')}）", 2)
# 表格与正文只列安装包；SHA256SUMS 等 .txt 也上传为附件（方便核对），但不进表格
rows_assets = [a for a in all_assets if not a["name"].endswith(".txt")]
if not rows_assets:
    die(f"{tag} 的产物里没有安装包（只有 {len(all_assets)} 个 .txt？CI 收集出了问题）", 2)

# 不镜像到 Gitee 的格式（用户 2026-11-17 拍板）：AppImage 自带整套 Linux 运行时，
# 84MB 里绝大部分是每次发版都重复传的运行时库，没意义。Gitee 正文保留该条目，
# 下载链接指向 GitHub Release；附件上传与回读核对都跳过它。
MIRROR_EXCLUDE = (".AppImage",)
def mirrored(n):
    return not n.endswith(MIRROR_EXCLUDE)

# 字节缓存：算 sha（老资产没 digest 时）与附件上传共用一次下载，不重复拉
_cache = {}
def asset_bytes(a):
    n = a["name"]
    if n in _cache:
        return _cache[n]
    req = urllib.request.Request(
        a.get("browser_download_url") or
        f"https://api.github.com/repos/{gh}/releases/assets/{a['id']}",
        headers={"Accept": "application/octet-stream", "User-Agent": "llm-switch-release-sync"})
    try:
        with urllib.request.urlopen(req, timeout=600) as r:
            data = r.read()
    except (urllib.error.HTTPError, urllib.error.URLError, TimeoutError, OSError) as e:
        classify(e, f"下载 GitHub 资产 {n}")
    _cache[n] = data
    return data

rows, blocks = [], []
for a in rows_assets:
    name = a["name"]
    size = f"{a.get('size', 0) / 1024 / 1024:.1f} MB"
    sha = (a.get("digest") or "").replace("sha256:", "")
    if not sha:  # 老资产没带 digest：下载回来本地算（单包几 MB，不心疼）
        sha = hashlib.sha256(asset_bytes(a)).hexdigest()
    gitee_dl = f"https://gitee.com/{ge}/releases/download/{tag}/{name}"
    if mirrored(name):
        dl = f"<{gitee_dl}>"
    else:
        # 不镜像的格式（AppImage）：条目保留，下载指 GitHub，说明原因
        dl = f"<{a.get('browser_download_url') or f'https://github.com/{gh}/releases/tag/{tag}'}>" \
             f"（体积大不镜像到 Gitee，GitHub 下载）"
    rows.append(f"| `{name}` | {platform_of(name)} | {size} |")
    blocks.append(f"**{name}**（{platform_of(name)}，{size}）\n\n"
                  f"下载：{dl}\n\nSHA256：`{sha}`")
    say(f"  {name} → {size} · sha256 {sha[:12]}…")

# ---------- 2. 定位 / 创建 Gitee Release ----------
say(f"同步到 Gitee {ge}")
rels = get(f"https://gitee.com/api/v5/repos/{ge}/releases?access_token={tok}&per_page=100") or []
exist = next((r["id"] for r in rels if r.get("tag_name") == tag), None)

payload = {
    # Gitee 要 snake_case：写成 tagName 会被判「tag_name is missing」（实测 400）。
    "access_token": tok, "tag_name": tag, "name": f"llm-switch {ver}",
}
if not exist:
    # 新建 Release 还必须给 target_commitish，否则 400「target_commitish is missing」
    # （实测 v0.2.48 首次 POST）。PATCH 已存在的 Release 不需要，故只在 POST 时带。
    payload["target_commitish"] = "main"

# ---------- 3. 附件同步（幂等：名字+大小对得上就跳过） ----------
def attach_list():
    out = get(f"https://gitee.com/api/v5/repos/{ge}/releases/{rid}/attach_files"
              f"?access_token={tok}&per_page=100")
    return [x for x in (out or []) if isinstance(x, dict) and x.get("name")]


def post_attach(filename, data):
    b = "----llmsw" + uuid.uuid4().hex
    body = b"".join([
        f'--{b}\r\nContent-Disposition: form-data; name="access_token"\r\n\r\n{tok}\r\n'.encode(),
        f'--{b}\r\nContent-Disposition: form-data; name="file"; filename="{filename}"\r\n'
        f'Content-Type: application/octet-stream\r\n\r\n'.encode(),
        data,
        f'\r\n--{b}--\r\n'.encode(),
    ])
    req = urllib.request.Request(
        f"https://gitee.com/api/v5/repos/{ge}/releases/{rid}/attach_files", data=body, method="POST")
    req.add_header("Content-Type", f"multipart/form-data; boundary={b}")
    try:
        with urllib.request.urlopen(req, timeout=1800) as r:
            return json.loads(r.read().decode())
    except (urllib.error.HTTPError, urllib.error.URLError, TimeoutError, OSError, json.JSONDecodeError) as e:
        classify(e, f"上传附件 {filename}")


def del_attach(aid):
    req = urllib.request.Request(
        f"https://gitee.com/api/v5/repos/{ge}/releases/{rid}/attach_files/{aid}?access_token={tok}",
        method="DELETE")
    try:
        with urllib.request.urlopen(req, timeout=60) as r:
            r.read()
    except (urllib.error.HTTPError, urllib.error.URLError, TimeoutError, OSError) as e:
        classify(e, f"删除旧附件 {aid}")


# ---------- 建 Release（附件要挂在它上面；已存在就先不动，正文最后统一写） ----------
rid = exist
if not exist:
    # Gitee 新建时 body 不能为空（实测 400「发行版的描述不能为空」），
    # 这里先放占位正文，附件传完后最后统一 PATCH 成真正文。
    payload["body"] = f"llm-switch {ver} 同步中……"
    data = urllib.parse.urlencode(payload).encode()
    req = urllib.request.Request(f"https://gitee.com/api/v5/repos/{ge}/releases", data=data,
                                 method="POST")
    req.add_header("Content-Type", "application/x-www-form-urlencoded")
    try:
        out = json.loads(urllib.request.urlopen(req, timeout=60).read().decode())
    except (urllib.error.HTTPError, urllib.error.URLError, TimeoutError, OSError, json.JSONDecodeError) as e:
        classify(e, "创建 Gitee Release")
    rid = out.get("id")
    say(f"已创建 Gitee Release id={rid} tag={out.get('tag_name')}")
else:
    say(f"Gitee Release 已存在 id={rid}，附件与正文按幂等补齐")

say("同步附件到 Gitee Release（幂等，只补缺失/大小不符的）")
byname = {x["name"]: x for x in attach_list()}
uploaded = 0
skipped_fmt = []
for a in all_assets:
    name, size = a["name"], int(a.get("size") or 0)
    if not mirrored(name):
        skipped_fmt.append(name)
        say(f"  按口径不镜像（跳过）：{name}")
        continue
    cur = byname.get(name)
    if cur and int(cur.get("size") or 0) == size:
        say(f"  已在，跳过：{name}")
        continue
    if cur:
        say(f"  大小不符（{cur.get('size')} != {size}），删除重传：{name}")
        del_attach(cur["id"])
    blob = asset_bytes(a)
    if len(blob) != size:
        die(f"{name} 下载到 {len(blob)} 字节 ≠ 资产 {size} 字节（GitHub 下载不完整）", 1)
    out = post_attach(name, blob)
    if int(out.get("size") or 0) != size:
        die(f"{name} 上传后大小不符（{out.get('size')} != {size}）", 1)
    uploaded += 1
    say(f"  已上传：{name}（{size / 1024 / 1024:.1f} MB）")
say(f"附件就位：本次新传 {uploaded} 个，资产共 {len(all_assets)} 个"
    + (f"（{len(skipped_fmt)} 个按口径不镜像：{', '.join(skipped_fmt)}）" if skipped_fmt else ""))

# ---------- 4. 正文（下载指向本页附件的 gitee 直链） ----------
notes = (os.environ.get("NOTES") or "").strip()
if "本版本无更新日志条目" in notes:
    notes = ""

body = (
    (f"### 本版本改动\n\n{notes}\n\n" if notes else "")
    + f"llm-switch {ver} 安装包由 GitHub Actions 出包，已同步上传为本页附件，"
      "直接从 Gitee 下载，SHA256 逐包核对。\n\n"
    "| 文件 | 平台 | 大小 |\n| --- | --- | --- |\n" + "\n".join(rows)
    + "\n\n---\n\n" + "\n\n".join(blocks)
    + "\n\n---\n\n"
    + f"GitHub Release 备份镜像：<https://github.com/{gh}/releases/tag/{tag}>。"
      "下载后请先核对 SHA256 再安装；两个公开仓（GitHub / Gitee）代码同源同 tag。"
)

patch = urllib.parse.urlencode(
    {"access_token": tok, "tag_name": tag, "name": f"llm-switch {ver}", "body": body}).encode()
req = urllib.request.Request(f"https://gitee.com/api/v5/repos/{ge}/releases/{rid}", data=patch,
                             method="PATCH")
req.add_header("Content-Type", "application/x-www-form-urlencoded")
try:
    json.loads(urllib.request.urlopen(req, timeout=60).read().decode())
except (urllib.error.HTTPError, urllib.error.URLError, TimeoutError, OSError, json.JSONDecodeError) as e:
    classify(e, "写入 Gitee Release 正文")
say("正文已写入（下载链接指向 gitee 本页附件）")

# ---------- 5. 回读核对：正文与附件都就位才算同步成（不符按瞬态交上层重试） ----------
back = get(f"https://gitee.com/api/v5/repos/{ge}/releases?access_token={tok}&per_page=100") or []
mine = next((r for r in back if r.get("tag_name") == tag), None)
if not mine or len(mine.get("body") or "") < len(body) // 2:
    die("回读 Gitee Release 正文对不上", 1)
back_attach = {x["name"]: int(x.get("size") or 0) for x in attach_list()}
missing = [a["name"] for a in all_assets
           if mirrored(a["name"]) and back_attach.get(a["name"]) != int(a.get("size") or 0)]
if missing:
    die(f"附件回读缺失或大小不符：{missing}", 1)
n_mirror = sum(1 for a in all_assets if mirrored(a["name"]))
say(f"回读核对通过：正文 {len(mine['body'])} 字，附件 {n_mirror}/{len(all_assets)} 个就位"
    + (f"（{len(all_assets) - n_mirror} 个按口径不镜像）" if n_mirror != len(all_assets) else ""))
PY
