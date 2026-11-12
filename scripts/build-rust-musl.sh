#!/usr/bin/env bash
# 构建并验证 Linux musl 完全静态产物（本地/CI 通用）。
#
# 依赖：rustup target、cargo-zigbuild、zig（cargo-zigbuild 会提示安装方式）。
# 用法：scripts/build-rust-musl.sh [target ...]
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ $# -gt 0 ]]; then
  targets=("$@")
else
  targets=(x86_64-unknown-linux-musl aarch64-unknown-linux-musl)
fi

if ! command -v cargo-zigbuild >/dev/null 2>&1; then
  cargo install cargo-zigbuild --locked
fi

for target in "${targets[@]}"; do
  rustup target add "$target"
  cargo zigbuild --locked --target "$target" --release --example mini_cli

  bin="target/$target/release/examples/mini_cli"
  file "$bin"
  if file "$bin" | grep -qE 'statically linked|static-pie linked'; then
    echo "[OK] $target is static"
  else
    echo "[FAIL] $target is not static"
    exit 1
  fi

  if command -v ldd >/dev/null 2>&1; then
    ldd "$bin" 2>&1 | grep -E 'not a dynamic executable|not found' || true
  fi
done
