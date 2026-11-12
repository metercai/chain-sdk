#!/usr/bin/env bash
# 角色矩阵与边界检查（S2 / S9 / S14 / S17 / S18 / S19 / S20）。
set -euo pipefail
cd "$(dirname "$0")/.."

cargo test --locked \
  --test role_gate \
  --test role_matrix \
  --test capabilities \
  --test keys_namespace \
  --test participant_binding \
  --test no_governance

echo "== S18: ChainSdk trait must not expose governance methods =="
if grep -rnE 'fn (register_participant|deactivate_participant|update_operator|register_anchor_type|submit_delegated_anchor)\b' src/; then
  echo "[FAIL] governance method found in src/ (belongs to chain-owner-sdk)"
  exit 1
fi

echo "== S17: ChainSdk trait must not expose VC storage methods =="
if grep -rnE 'fn (store_vc|get_vc|list_vcs|remove_vc)\b' src/; then
  echo "[FAIL] VC storage method found in src/ (belongs to did-sdk)"
  exit 1
fi

echo "[OK] role boundary checks passed"
