#!/usr/bin/env bash
# Mock vs Centralized 语义一致性（S3 / S12）。
set -euo pipefail
cd "$(dirname "$0")/.."

cargo test --locked --test consistency --test role_gate -- --nocapture
