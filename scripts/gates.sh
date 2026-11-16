#!/usr/bin/env bash
# 一条命令跑完本仓库的全部门禁（每条切片收尾都必须全绿才能提交）。
# 为什么要有这个脚本：命令散在文档里，跑到第三条漏一条是常事；浮窗那种
# 只在 `--features gui` 下编译的代码，跑默认 `cargo test` 根本不会被编到。
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root/src-tauri"

echo "── rustfmt"
cargo fmt --all -- --check

echo "── clippy (默认特性)"
cargo clippy --all-targets -- -D warnings

echo "── clippy (--features gui)"
cargo clippy --all-targets --features gui -- -D warnings

echo "── cargo test"
cargo test

echo "── cargo test --features gui --lib（浮窗等桌面壳专属代码）"
cargo test --features gui --lib

echo "── ui build"
(cd "$root/ui" && npm run build)

echo "── ui lint"
(cd "$root/ui" && npm run lint)

echo "── ui 债务预算"
node "$root/scripts/ui-budget.mjs"

echo "✓ 全部门禁通过"
