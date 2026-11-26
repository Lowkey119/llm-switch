#!/usr/bin/env bash
# 取 CHANGELOG.md 里某个版本的小节（不含 `## v…` 标题行）。
# 用法：scripts/release-notes.sh <tag|version> [changelog 路径]
#   scripts/release-notes.sh v0.2.57
#   scripts/release-notes.sh 0.2.57 docs/CHANGELOG.md
#
# 为什么单独成脚本：GitHub Release 正文（.github/workflows/release.yml）与
# Gitee Release 正文（scripts/gitee-release-sync.sh）必须给同一份更新说明，
# 两处各写一遍 awk 迟早分叉。本脚本在公开仓白名单里，CI 两个 job 都能用。
#
# 没有条目时打印占位并退 0——补跑老版本（CHANGELOG 建立之前的 tag）属正常情况，
# 不该因此把出包流程弄红。
set -euo pipefail

VER="${1:?用法: release-notes.sh <tag|version> [changelog]}"
VER="${VER#v}"
FILE="${2:-CHANGELOG.md}"

if [ ! -f "$FILE" ]; then
  echo "（本仓库未附更新日志）"
  exit 0
fi

out="$(awk -v v="## v$VER" '
  found && /^## / { exit }
  $0 == v { found = 1; next }
  found { print }
' "$FILE")"

if [ -z "${out//[[:space:]]/}" ]; then
  echo "（本版本无更新日志条目）"
else
  printf '%s\n' "$out"
fi
