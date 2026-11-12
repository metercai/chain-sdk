# chain-sdk 验收清单

> 状态：**初始化骨架（2026-11-13）**。S1–S20 与 SD-JWT 项目前为「待实施」；
> 每个阶段完成后在此填写命令、结果摘要与提交号。
>
> 规格来源：`docs/specs/01-...`（S1–S20）、`docs/specs/02-...`（R1）、
> `docs/specs/03-...`（SD-JWT I/D/V/P）。

---

## 0. 基础环境

| 项 | 期望 | 现状 |
|---|---|---|
| 工具链 | `rustc 1.88.0` | ✅ 1.88.0 |
| Edition | 2024 | ✅ |
| `resolver` | 3 | ✅ |
| `Cargo.lock` | 已提交、`--locked` 可解析 | ✅ 已生成（472 包） |
| 本地 A1 | `cargo check --locked --all-targets --all-features` | ✅ 通过（44.7s） |
| 本地 A2 | `cargo fmt --all -- --check` | ✅ 通过 |
| 本地 A2 | `cargo clippy --locked --all-targets --all-features -- -D warnings` | ✅ 零告警 |
| 本地 A3 | `cargo test --locked --all-features` | ✅ 49 tests（P0–P4） |
| 依赖边界 | 无 `openssl` / `native-tls` | ✅ |
| R1 边界 | `did-sdk` 依赖树无 `subxt` | ✅ 0 处 |
| GitHub | 仓库创建 + 4 workflow 全绿 | ⏳ 待 D-18 |

---

## 1. 契约与后端（S1–S20）

| 编号 | 验收项 | 通过标准 | 状态 | 证据 |
|---|---|---|---|---|
| S1 | 接口契约完整 | `ChainSdk` 16 方法齐备；`cargo check` | ⏳ 待实施 | `src/lib.rs`；CI 契约步骤 |
| S2 | 三后端模式切换 | `mock`/`centralized`/`chain` 经 `mode` 切换，需重启 | 🟡 配置解析通过；后端待 P1/P5/P10 | `tests/config_resolution.rs`（10 用例） |
| S3 | 三后端语义一致 | 同一操作结果与错误码一致 | ⏳ | `tests/consistency.rs` |
| S4 | Mock 状态预置与持久化 | 预置可读；写后 JSON 更新；重启仍在 | ✅ P1/P2 | `tests/mock_persistence.rs`（4）+ `tests/role_gate.rs::writes_are_persisted` |
| S5 | Mock 故障注入 | 延迟 / 不可用 / 指定错误 | ✅ P1 | `tests/mock_fault_injection.rs`（4） |
| S6 | Centralized 接口映射 | method + path + 查询参数正确 | ⏳ | `tests/centralized_contract.rs` |
| S7 | Centralized 写签名 | `x-chain-*` 三头；签名可验 | ⏳ | `tests/centralized_signing.rs` |
| S8 | Centralized 缓存 | TTL 命中 + 过期穿透 + 关闭 | ⏳ | `tests/centralized_cache.rs` |
| S9 | Chain 角色拦截 | Verifier 写 → `Unauthorized`；断言 → `SignerUnavailable` | ✅ P2（Mock） | `tests/role_gate.rs::verifier_cannot_write_any_backend` |
| S10 | 私钥管理内置 | FileKeystore 可用；密码链正确 | 🟡 FileKeystore 签名已通过；完整密码链待 P4 | `tests/keys_namespace.rs::issuer_signs_with_file_keystore` |
| S11 | 事件订阅 | 三后端均能收到事件；取消幂等 | ⏳ | `tests/event_subscription.rs` |
| S12 | 一致性测试 | Mock vs Centralized 同结果 | ⏳ | `tests/consistency.rs` |
| S13 | `keyvault-core` 集成 | `UserRejected` / namespace 错误映射 | ⏳ | `tests/keyvault_integration.rs` |
| **S14** | **namespace 隔离** | 错误前缀 → `KeyNamespaceViolation` | ✅ P0 | `cargo test --test keys_namespace`（7 用例） |
| **S15** | **参与方身份绑定** | DID ↔ id ↔ operator 双向索引 | 🟡 索引读/迁移已通过；fixtures 全量待 P7 | `tests/role_gate.rs::issuer_updates_own_controller_did_and_index` |
| **S16** | **委托锚定语义** | `issuer_did` 信任根 / `submitter_id` / `delegated_by_did` | ⏳ | `tests/participant_binding.rs` |
| **S17** | **按角色装配** | Verifier 无 vault / 有 did；Issuer 两者均有 | ✅ P4 | `tests/capabilities.rs`（5）+ `src/config.rs` 单元测试（3） |
| **S18** | **无治理接口** | trait 无 5 个治理方法；CI grep | ⏳ | `tests/no_governance.rs`；`scripts/verify-roles.sh` |
| **S19** | **自身管理写越权** | 改他人 → `Unauthorized` | ✅ P2 | `tests/role_gate.rs`（10） |
| **S20** | **business_types 不参与鉴权** | 篡改后行为不变 | ⏳ | `tests/participant_binding.rs` |
| S1-static | musl 静态链接 | `file` 断言 static；`ldd` 无动态依赖 | ⏳ CI | `rust-musl.yml` |
| S2-UserRejected | `UserRejected` 透传 | `KeyVaultError::UserRejected` → `ChainSdkError::UserRejected` | ⏳ | `tests/keyvault_integration.rs` |
| S3-AppAlias | `role = "app"` 兼容 | 反序列化为 `Issuer` | ✅ P0 | `tests/config_resolution.rs::role_app_alias_maps_to_issuer` |

---

## 2. 平台与构建

| 编号 | 项 | 通过标准 | 状态 | 证据 |
|---|---|---|---|---|
| P1 | `x86_64-unknown-linux-musl` | 完全静态 | ⏳ CI | `rust-musl.yml` |
| P2 | `aarch64-unknown-linux-musl` | 完全静态 | ⏳ CI | 同上 |
| P3 | `aarch64-apple-darwin` | 原生构建 | ⏳ CI | 同上 |
| P4 | `x86_64-pc-windows-msvc` | 原生构建 | ⏳ CI | 同上 |
| P5 | 产物上传 | `chain-sdk-mini-cli-<target>` artifact | ⏳ CI | 同上 |

---

## 3. VC 选择性披露（SD-JWT，条件交付）

### 3.1 签发侧（chain-sdk Issuer）

| 编号 | 验收项 | 通过标准 | 状态 |
|---|---|---|---|
| I1 | 盐化哈希生成 | 每属性 16B 随机盐；`sd-v1` 公式 | ⏳ |
| I2 | `_sd` 数组构造 | VC 含正确 `_sd` / `_sd_alg` | ⏳ |
| I3 | 独立随机盐 | 同 VC 不同属性、不同 VC 同属性哈希均不同 | ⏳ |
| I4 | Disclosures 生成 | `[salt, name, value]` 格式 | ⏳ |
| I5 | 链上锚定不变 | `payload_hash` 覆盖 VC（含 `_sd`，不含 Disclosures） | ⏳ |
| I6 | Disclosures 交付 | 与 VC 一并安全交付（依赖 did-sdk v0.2） | ⏳ 外部依赖 |

### 3.2 出示侧（did-sdk，跨仓库）

| 编号 | 验收项 | 状态 |
|---|---|---|
| D1 | Disclosures 与 VC 绑定加密存储 | ⏳ 外部依赖 |
| D2 | 用户逐项选择（默认不勾选） | ⏳ 外部依赖 |
| D3 | 只发送选中子集 | ⏳ 外部依赖 |
| D4 | VP 含 VC + `disclosures` + holder 签名 | ⏳ 外部依赖 |
| D5 | 持有者签名覆盖 VC + disclosures + challenge + domain | ⏳ 外部依赖 |

### 3.3 验证侧（chain-sdk Verifier）

| 编号 | 验收项 | 通过标准 | 状态 |
|---|---|---|---|
| V1 | VP 签名验证 | holder 签名有效；challenge 匹配 | ⏳ |
| V2 | VC 签名验证 | issuer 签名有效 | ⏳ |
| V3 | Disclosure 哈希比对 | 每条重算命中 `_sd` | ⏳ |
| V4 | 未披露属性不可见 | 仅见 `_sd` 哈希 | ⏳ |
| V5 | 链上锚定查询 | `revoked` / `valid_until` 检查 | ⏳ |
| V6 | 篡改检测 | 改盐/值 → `DisclosureHashMismatch` | ⏳ |
| V7 | 重复披露检测 | 同名重复 → `DisclosureDuplicate` | ⏳ |

### 3.4 隐私边界

| 编号 | 验收项 | 通过标准 | 状态 |
|---|---|---|---|
| P-SD1 | 跨 VC 不可关联 | 同主体不同 VC 同属性哈希不同 | ⏳ |
| P-SD2 | 同 VC 跨出示可关联 | 文档明确标注边界 | ✅ README/SD 附件 |
| P-SD3 | 字典攻击防护 | 哈希不可反推 | ⏳ |

---

## 4. GitHub Actions

| workflow | 触发 | 状态 |
|---|---|---|
| `ci.yml` | push / PR / dispatch | ⏳ 待仓库创建 |
| `rust-musl.yml` | push / PR / dispatch | ⏳ |
| `consistency.yml` | push / PR / dispatch | ⏳ |
| `roles.yml` | push / PR / dispatch | ⏳ |

---

## 5. 本地测试计数（每阶段更新）

| 阶段 | 新增测试 | 累计 | 结果 |
|---|---|---|---|
| 初始化 | 0 | 0 | ✅ 0 passed |
| P0 | 17 | 17 | ✅ 17 passed（config_resolution 10 + keys_namespace 7） |
| P1 | 8 | 25 | ✅ mock_persistence 4 + mock_fault_injection 4 |
| P2 | 10 | 41 | ✅ role_gate 10 |
| P3 | 0（并入 P0） | 41 | ✅ S14 已通过 |
| P4 | 8 | 49 | ✅ capabilities 5 + config 单元 3 |
| P5 | — | — | ⏳ |
| P6 | — | — | ⏳ |
| P7 | — | — | ⏳ |
| P8 | — | — | ⏳ |
| P9 | — | — | ⏳ |
| P10 | — | — | ⏳ |
| P11 | — | — | ⏳ |
| P12（可选） | — | — | ⏳ |

---

## 6. 待外部条件

| 项 | 需要 | 影响 |
|---|---|---|
| GitHub 仓库 | 用户创建 `metercai/chain-sdk` + PAT/SSH 授权（D-18） | A11、全部 CI |
| 中心化模拟链契约 | Q-01 | S6 / S7 |
| identity-chain 元数据 / 节点 | Q-02 | Chain 真实链验收（R-01） |
| did-sdk v0.2 | D-16 | SD-JWT 端到端（I6 / D1–D5） |

---

## 7. 变更记录

| 日期 | 内容 |
|---|---|
| 2026-11-13 | 初始化：骨架 + 契约 + CI；S1–S20 待实施 |
