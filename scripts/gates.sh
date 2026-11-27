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

# 跨平台移植 S-X1：遥控器平台层要能编到 Win/Linux。本机没有交叉 C 工具链与
# webkit/alsa 系统库，那种失败属「环境不具备」，提示后跳过；本仓代码本身编不过必须红。
cross_check() {
  target="$1"
  if command -v rustup >/dev/null 2>&1 && ! rustup target list --installed 2>/dev/null | grep -qx "$target"; then
    echo "   跳过  $target 未安装（rustup target add $target）"
    return 0
  fi
  log="$(mktemp)"
  if cargo check --target "$target" --features gui --lib >"$log" 2>&1; then
    echo "   ok    $target"
    rm -f "$log"
    return 0
  fi
  if grep -qE "pkg-config has not been configured|could not find .pkg-config|error occurred in cc-rs|error linking|linker .* not found|could not find system library" "$log"; then
    echo "   跳过  $target 缺交叉 C 工具链/系统库（CI 上会真编）"
    rm -f "$log"
    return 0
  fi
  echo "   失败  $target"
  tail -40 "$log"
  rm -f "$log"
  return 1
}

echo "── 跨编译（遥控器平台层）"
cross_check x86_64-pc-windows-msvc
cross_check x86_64-unknown-linux-gnu

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
