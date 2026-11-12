# ChainSdk 项目实施文档（Agent 可直接执行，v2）

> 用途：本文件是自包含的项目引导规格。把整份文件作为输入交给代码生成 agent，
> 要求其在空目录中创建 GitHub 仓库 `chain-sdk`，产出一套 Rust 产物：
> **`chain-sdk` 单一 crate**，实现 `Mock` / `Centralized` / `Chain` 三种后端与
> `Verifier` / `Issuer` 两种链上角色的统一契约。
> **本版基于《02-ChainSdk 接口契约与实现规格书》最新版**：两角色线性包含、
> 两类服务端密钥（namespace 隔离）、按角色装配 did-sdk 能力子集、
> 参与方身份三段式硬绑定、委托锚定语义、S1–S20 验收清单。
>
> **平台与静态链接策略（与 did-sdk 对齐）**
>
> - **Rust 产物**：目标为 `x86_64-unknown-linux-musl`、`aarch64-unknown-linux-musl`、
>   `aarch64-apple-darwin`、`x86_64-pc-windows-msvc`。Linux musl 产物要求
>   **完全静态、零运行时依赖**；因此 Rust 侧禁止任何未显式静态化的 `-sys` 依赖，
>   TLS 统一用 `rustls`（纯 Rust），HTTP 用 `reqwest(rustls-tls)`。
> - **不提供 Python / Node / WASM 绑定**：`chain-sdk` 是服务端 Rust 应用的契约。
> - **`ChainOwner` 不在本 crate 中**：`register_participant` / `deactivate_participant` /
>   `update_operator` / `register_anchor_type` / `submit_delegated_anchor` 属于
>   `chain-owner-sdk`；`ChainSdk` trait **不得**包含这些方法（S18）。
>
> **外部依赖（不重复实现）**
>
> - **`keyvault-core`**：由 `did-sdk` 仓库发布的子 crate，直接依赖，不在本仓库
>   复制或重写。
> - **`did-sdk`**：`chain-sdk` 按角色装配其能力子集（`did-resolution` /
>   `vc-wallet` / `issuer-signing`）；VC 存储由 `did-sdk` 管理，`chain-sdk`
>   不提供 VC 存储接口（S17）。
>
> **`UserRejected` 变体预留**：`ChainSdkError` 预留 `UserRejected`，与
> `keyvault-core::KeyVaultError::UserRejected` 对齐，1:1 透传。

---

## 0. 总览

### 0.1 仓库名与 crate 关系

- GitHub 仓库名：`chain-sdk`；根目录**同时是** Rust crate `chain-sdk` 的
  `[package]` 与 `[workspace]` 根。
- **`keyvault-core` 与 `did-sdk` 均为外部依赖**，不在本仓库内。
- **`chain-owner-sdk`**：依赖本 crate，扩展治理接口；本 crate 不感知。
- **VC 存储不在本 crate**：由 `did-sdk<Server>` 持有侧管理，`chain-sdk` 只提供
  链上锚定查询与 `sign_participant_assertion` 签名原语。

### 0.2 外部依赖引入方式

```toml
[workspace.dependencies]
keyvault-core = { git = "https://github.com/<org>/did-sdk", tag = "v0.1.0", package = "keyvault-core" }
did-sdk       = { git = "https://github.com/<org>/did-sdk", tag = "v0.1.0" }
```

**本地联调**：追加 `[patch."https://github.com/<org>/did-sdk"]` 指向本地路径；
**未来 crates.io 发布**：改为 `version = "0.1.0"`。三种方式对 `chain-sdk` 源码零影响。

### 0.3 角色模型（**重要**）

`chain-sdk` 只暴露**两种配置角色**：`SdkRole::Verifier` 与 `SdkRole::Issuer`。

- **线性包含**：`Verifier ⊂ Issuer`。Issuer 天然包含 Verifier 的全部读能力；
  不存在"纯 Issuer 不能验证"的情况。
- **旧配置 `role = "app"`**：映射为 `Issuer`（`#[serde(alias = "app")]`），向后兼容。
- **`ChainRole`**（链上角色，`ParticipantRecord.chain_role` 字段）与
  **`SdkRole`**（SDK 构造配置）是两个概念：
  - `ChainRole`：`Verifier` / `Issuer` / `ChainOwner`，是链上注册表字段，
    由链主治理分配，**决定链上能力边界**；
  - `SdkRole`：SDK 实例化时选择的角色，**决定本进程加载哪些密钥与能力**。

### 0.4 三后端 × 两角色矩阵

| 模式 \ 角色 | `Verifier` | `Issuer` |
|---|---|---|
| `Mock` | 内存读；无 `KeyVault`；`sign_participant_assertion` 返回 `SignerUnavailable` | 内存读写；两类密钥（sr25519 + ed25519） |
| `Centralized` | HTTP 只读；无 `KeyVault` | HTTP 读写；链账户签名写请求 + did:web 断言签名 |
| `Chain` | `subxt` storage 只读；无 `KeyVault` | `subxt` 读写；sr25519 签 extrinsic + ed25519 签断言 |

**越权调用**：`Verifier` 调任何写方法或签名方法，返回 `Unauthorized` /
`SignerUnavailable`（三后端一致）。

### 0.5 能力装配矩阵（S17）

| 角色 | `did-resolution` | `vc-wallet` | `issuer-signing` | KeyVault | 写操作 |
|---|---|---|---|---|---|
| Verifier | 是 | 可选 | 否 | 不加载 | 无 |
| Issuer | 是 | 可选 | 是 | 加载两类密钥 | 锚定写 + 自身管理写 + 断言签名 |

---

## 1. 目录树

```text
chain-sdk/
├── Cargo.toml
├── rust-toolchain.toml
├── README.md
├── .gitignore
├── src/
│   ├── lib.rs                      # ChainSdk trait + build_chain_sdk 工厂
│   ├── config.rs                   # ChainSdkConfig / SdkRole / ChainSdkMode / Capabilities
│   ├── types.rs                    # ParticipantRecord / AnchorRecord / DidRecord 等
│   ├── error.rs                    # ChainSdkError（含 UserRejected / KeyNamespaceViolation）
│   ├── role_gate.rs                # 角色能力检查（ensure_issuer / ensure_self / ensure_signer）
│   ├── keys.rs                     # 两类密钥 namespace 管理 + 前缀校验
│   ├── events.rs                   # Unsubscribe / EventData
│   ├── did_adapter.rs              # 按角色装配 did-sdk 能力子集
│   ├── mock.rs                     # MockChainSdk
│   ├── centralized.rs              # CentralizedChainSdk
│   └── chain.rs                    # ChainModeSdk（subxt）
├── metadata/
│   └── identity_chain.scale
├── examples/
│   └── mini_cli.rs
├── tests/
│   ├── common/mod.rs
│   ├── config_resolution.rs
│   ├── role_gate.rs                # S9 / S19
│   ├── role_matrix.rs              # S2
│   ├── capabilities.rs             # S17
│   ├── keys_namespace.rs           # S14
│   ├── participant_binding.rs      # S15 / S16 / S20
│   ├── no_governance.rs            # S18
│   ├── consistency.rs              # S3 / S12
│   ├── mock_persistence.rs         # S4
│   ├── mock_fault_injection.rs     # S5
│   ├── centralized_contract.rs     # S6
│   ├── centralized_signing.rs      # S7
│   ├── centralized_cache.rs        # S8
│   ├── keyvault_integration.rs     # S10 / S13
│   ├── event_subscription.rs       # S11
│   ├── configs/
│   │   ├── verifier.toml
│   │   └── issuer.toml
│   └── fixtures/
│       ├── mock_state.json
│       └── delegated_anchor.json
├── configs/
│   └── chain-sdk.toml.example
├── scripts/
│   ├── build-rust-musl.sh
│   ├── run-consistency.sh
│   └── verify-roles.sh
├── .github/workflows/
│   ├── ci.yml
│   ├── rust-musl.yml
│   ├── consistency.yml
│   └── roles.yml
└── docs/
    └── acceptance-checklist.md
```

---

## 2. 工作区根 `Cargo.toml`

```toml
[package]
name = "chain-sdk"
version = "0.2.0"
edition = "2024"
rust-version = "1.85"
license = "Apache-2.0"
description = "ChainSdk: chain-capability contract (Mock/Centralized/Chain) for Verifier/Issuer with two service-side key namespaces"

[workspace]
resolver = "2"
members = ["."]
default-members = ["."]

[workspace.package]
version = "0.2.0"
edition = "2024"
rust-version = "1.85"
license = "Apache-2.0"
repository = "https://github.com/<org>/chain-sdk"

[workspace.dependencies]
# 外部依赖：由 did-sdk 仓库发布；pin tag 保证可重现构建
keyvault-core = { git = "https://github.com/<org>/did-sdk", tag = "v0.1.0", package = "keyvault-core" }
did-sdk       = { git = "https://github.com/<org>/did-sdk", tag = "v0.1.0" }

async-trait = "0.1.91"
async-lock = "3"
futures = "0.3"

serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"
thiserror = "2"
tracing = "0.1"
secrecy = { version = "0.10", features = ["zeroize"] }
hex = "0.4"
dirs = "6"

tokio = { version = "1.44", features = ["rt-multi-thread", "macros", "sync", "fs", "time"] }
# musl 静态：必须 rustls，禁止 native-tls / openssl
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
tokio-tungstenite = { version = "0.24", default-features = false, features = ["rustls-tls-webpki-roots"] }
subxt = "0.50"

wiremock = "0.6"
tempfile = "3"
serial_test = "3"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

[dependencies]
keyvault-core = { workspace = true }
did-sdk = { workspace = true }
async-trait = { workspace = true }
async-lock = { workspace = true }
futures = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
toml = { workspace = true }
thiserror = { workspace = true }
tracing = { workspace = true }
secrecy = { workspace = true }
hex = { workspace = true }
tokio = { workspace = true }
reqwest = { workspace = true }
tokio-tungstenite = { workspace = true }
subxt = { workspace = true }
dirs = { workspace = true }

[dev-dependencies]
wiremock = { workspace = true }
tempfile = { workspace = true }
serial_test = { workspace = true }
tokio = { workspace = true, features = ["full"] }
tracing-subscriber = { workspace = true }

[features]
default = []
# 能力子集（S17）：仅做 feature 门控，避免 Verifier 拉入全量依赖
did-resolution = []
vc-wallet = []
issuer-signing = []
# 未来密钥后端扩展点
device-keystore = []
server-keystore = []

[[example]]
name = "mini_cli"
path = "examples/mini_cli.rs"

[lints.rust]
unsafe_code = "forbid"
missing_docs = "warn"

[lints.clippy]
unwrap_used = "warn"
expect_used = "warn"
```

`rust-toolchain.toml`：与 v1 相同（四平台 + clippy + rustfmt）。

**musl 静态链接约束**：`reqwest` 必须 `rustls-tls`；`tokio-tungstenite` 必须
`rustls-tls-webpki-roots`；`subxt` 纯 Rust；`did-sdk` / `keyvault-core`
传递依赖均为纯 Rust 密码学。禁止 `openssl` / `native-tls` / 无 bundled 的 `-sys`。

---

## 3. `keyvault-core` 与 `did-sdk` 依赖（**不重复实现**）

### 3.1 `keyvault-core` 消费的 API

| API | 用途 |
|---|---|
| `KeyVault` trait | `SdkCore.vault: Option<Arc<dyn KeyVault>>` 的类型 |
| `FileKeystore::new(path, password)` | `scheme = "file-keystore"` 时构造 |
| `MemoryKeystore::new()` | `scheme = "memory-keystore"`（测试 / Mock 免配置） |
| `KeyVaultError` | 错误统一映射，含 `UserRejected` |
| `PasswordSource` / `resolve_password` | 密码解析（Env / File / Direct / AutoGenerate） |
| `KeyId` / `Signature` | `KeyId` 字符串前缀表达 namespace；`Signature` 是签名字节 |

### 3.2 namespace 实现策略（S14）

`keyvault-core::FileKeystore` 的物理存储按 `index.json` 索引，路径为
`./keystore/key-<id>.enc`。**`chain-sdk` 不修改 keyvault-core**，而是：

- **命名约定**：`KeyId.0` 以 `chain-account/` 或 `participant-did/` 开头；
- **逻辑隔离**：`chain-sdk::keys` 模块提供两个显式构造函数：
  - `load_chain_account_key(vault, "chain-account/operator-1")`
  - `load_participant_did_key(vault, "participant-did/did_web_...")`
- **前缀校验**：`sign_extrinsic` / `sign_participant_assertion` 在调用
  `vault.sign` 之前校验 `key_id.0.starts_with(...)`，不匹配返回
  `ChainSdkError::KeyNamespaceViolation`；
- **物理分目录**（可选）：若未来 `keyvault-core` 扩展 prefix 分目录，只需
  改 `keys.rs` 的构造逻辑，不影响 `ChainSdk` trait 与所有 impl。

`KeyVaultConfig.keys` 提供两把 key 的 `KeyId` 映射：

```rust
pub struct KeyVaultKeys {
    pub operator: String,            // "chain-account/operator-1"
    pub participant_did: String,     // "participant-did/did_web_..."
}
```

### 3.3 `did-sdk` 消费的 API（按角色装配）

`chain-sdk` 在 `SdkCore.did` 中持有 `Option<Arc<dyn DidSdk>>`。
**按角色装配**（S17）：

| 能力 | Verifier | Issuer | 配置项 |
|---|---|---|---|
| `did-resolution` | 是 | 是 | `capabilities.did_resolution = true` |
| `vc-wallet` | 可选 | 可选 | `capabilities.vc_wallet = false`（POC 默认关闭） |
| `issuer-signing` | **否** | **是** | `capabilities.issuer_signing = true` |

`did-sdk` 侧需提供的最小接口（以 `did-sdk` 实际 `DidSdk` trait 为准）：

- `resolve_did_key(did) -> Ed25519PublicKey`（同步）
- `resolve_did_web(did) -> DidDocument`（异步）
- `resolve_did(did) -> Option<DidRecord>`（异步，链上 DID）
- 内部 `Signer` 注入用于 `issuer-signing` 能力：`chain-sdk` 的
  `sign_participant_assertion` 通过 `did-sdk` 暴露的签发接口产生 `Proof`，
  再交给 `did-sdk.assemble_vc_with_proof` 组装 VC。
- **VC 存储接口不存在于 `chain-sdk`**（S17）：所有 VC 存储、列出、删除都由
  `did-sdk` 负责。

若 `did-sdk` 的 `Server` 模式接口需要签名器注入，`chain-sdk` 把参与方
`did:web` 私钥（`participant-did/` namespace）包装为 `did-sdk` 认可的
`Signer` 注入。**详细接口以 `did-sdk` 最新版本为准**；本文件只约定
`chain-sdk` 需要的边界。

---

## 4. `chain-sdk` 根 crate

### 4.1 `src/lib.rs`

```rust
//! chain-sdk: 链能力契约（Mock/Centralized/Chain × Verifier/Issuer）。
//!
//! **两角色线性包含**：`Verifier ⊂ Issuer`。Issuer 天然包含 Verifier 全部读能力。
//! 写方法（`create_anchor` 等）仅 Issuer 可调用；`Verifier` 调用返回 `Unauthorized`。
//! `sign_participant_assertion` 仅 Issuer 可调用；`Verifier` 调用返回 `SignerUnavailable`。
//!
//! **两类服务端密钥、两个 namespace**（S14）：
//! - `chain-account/`：sr25519，extrinsic / HTTP 写请求体签名（`sign_extrinsic`）
//! - `participant-did/`：ed25519，参与方 did:web 断言签名（`sign_participant_assertion`）
//!
//! **VC 存储不在本 crate**：由 `did-sdk<Server>` 管理。

pub mod config;
pub mod error;
pub mod types;

mod chain;
mod centralized;
mod did_adapter;
mod events;
mod keys;
mod mock;
mod role_gate;

use std::sync::Arc;
use async_trait::async_trait;
use keyvault_core::KeyVault;

pub use config::{build_chain_sdk, ChainSdkConfig, ChainSdkMode, SdkRole};
pub use error::ChainSdkError;
pub use events::{EventData, Unsubscribe};
pub use types::{
    AnchorRecord, AnchorType, AssertionPurpose, BusinessType, ChainRole, DidDocument,
    DidRecord, Ed25519PublicKey, Ed25519Signature, HealthStatus, ParticipantRecord,
    Sr25519Signature,
};

#[async_trait]
pub trait ChainSdk: Send + Sync {
    // ===== 连接管理 =====
    async fn health_check(&self) -> Result<HealthStatus, ChainSdkError>;

    // ===== 锚定读（两角色均可） =====
    async fn get_anchor(&self, anchor_id: &str) -> Result<AnchorRecord, ChainSdkError>;
    async fn list_anchors(
        &self,
        anchor_type: Option<AnchorType>,
        submitter_id: Option<u32>,
    ) -> Result<Vec<AnchorRecord>, ChainSdkError>;

    // ===== 参与方读（两角色均可） =====
    async fn get_participant(&self, participant_id: u32) -> Result<ParticipantRecord, ChainSdkError>;
    async fn get_participant_by_did(&self, did: &str) -> Result<Option<ParticipantRecord>, ChainSdkError>;
    async fn list_participants(&self) -> Result<Vec<ParticipantRecord>, ChainSdkError>;

    // ===== DID（两角色均可） =====
    async fn resolve_did(&self, did: &str) -> Result<Option<DidRecord>, ChainSdkError>;
    fn resolve_did_key(&self, did: &str) -> Result<Ed25519PublicKey, ChainSdkError>;
    async fn resolve_did_web(&self, did: &str) -> Result<DidDocument, ChainSdkError>;

    // ===== 锚定写（仅 Issuer） =====
    async fn create_anchor(
        &self,
        anchor_type: AnchorType,
        payload_hash: &str,
        valid_until: u64,
    ) -> Result<String, ChainSdkError>;
    async fn revoke_anchor(&self, anchor_id: &str) -> Result<(), ChainSdkError>;
    async fn renew_anchor(&self, anchor_id: &str, new_valid_until: u64) -> Result<(), ChainSdkError>;

    // ===== 参与方自身管理写（仅 Issuer，且仅自身） =====
    async fn update_authorized_domains(
        &self,
        participant_id: u32,
        domains: Vec<String>,
    ) -> Result<(), ChainSdkError>;
    async fn update_controller_did(
        &self,
        participant_id: u32,
        new_controller_did: &str,
    ) -> Result<(), ChainSdkError>;

    // ===== 参与方断言签名（仅 Issuer；Server 模式 did-sdk 注入 Signer） =====
    async fn sign_participant_assertion(
        &self,
        payload: &[u8],
        purpose: AssertionPurpose,
    ) -> Result<Ed25519Signature, ChainSdkError>;

    // ===== 事件订阅（两角色均可） =====
    fn subscribe_event(
        &self,
        event_name: &str,
        callback: Box<dyn Fn(EventData) + Send + Sync>,
    ) -> Result<Unsubscribe, ChainSdkError>;
}

/// 供三个后端实现共用的构造参数。
pub(crate) struct SdkCore {
    /// Verifier: None；Issuer: Some（加载两类密钥）
    pub vault: Option<Arc<dyn KeyVault>>,
    /// 两角色都装配 did-sdk（did-resolution 对两角色都需要）；vc-wallet 可选
    pub did: Option<Arc<dyn did_sdk::DidSdk>>,
    pub config: ChainSdkConfig,
    pub role: SdkRole,
    /// 两类密钥的 KeyId（Issuer 才有值）
    pub keys: Option<crate::keys::LoadedKeys>,
}

/// `KeyVaultError` → `ChainSdkError` 映射，**所有 impl 必须使用此函数**
pub(crate) fn map_vault_err(e: keyvault_core::KeyVaultError) -> ChainSdkError {
    use keyvault_core::KeyVaultError as KE;
    match e {
        KE::KeyNotFound(s) => ChainSdkError::SignerUnavailable(format!("key not found: {s}")),
        KE::UserRejected => ChainSdkError::UserRejected,
        KE::InvalidPassword => ChainSdkError::SignerUnavailable("invalid keystore password".into()),
        KE::Locked => ChainSdkError::SignerUnavailable("keystore locked".into()),
        KE::SigningFailed(s) => ChainSdkError::SigningFailed(s),
        KE::IoError(e) => ChainSdkError::ChainUnavailable(e.to_string()),
        KE::InvalidKeyFormat(s) => ChainSdkError::InvalidArgument(s),
        KE::DecryptionFailed => ChainSdkError::SigningFailed("decryption failed".into()),
        KE::SerializationError(e) => ChainSdkError::InvalidArgument(e.to_string()),
        KE::Unsupported => ChainSdkError::InvalidArgument("unsupported keystore op".into()),
        KE::KeyAlreadyExists(s) => ChainSdkError::InvalidArgument(format!("key exists: {s}")),
    }
}
```

### 4.2 `src/error.rs`

```rust
use crate::types::AnchorType;

#[derive(Debug, thiserror::Error)]
pub enum ChainSdkError {
    #[error("Anchor not found: {0}")] AnchorNotFound(String),
    #[error("Anchor type not registered: {0:?}")] AnchorTypeNotRegistered(AnchorType),
    #[error("Anchor is permanent: {0}")] AnchorPermanent(String),
    #[error("Anchor already revoked: {0}")] AnchorRevoked(String),
    #[error("Participant not found: {0}")] ParticipantNotFound(u32),
    #[error("Participant DID not found: {0}")] ParticipantDidNotFound(String),
    #[error("Participant inactive: {0}")] ParticipantInactive(u32),
    #[error("Unauthorized: {0}")] Unauthorized(String),
    #[error("Key namespace violation: {0}")] KeyNamespaceViolation(String),
    #[error("Signer not available in role: {0}")] SignerUnavailable(String),
    #[error("Chain unavailable: {0}")] ChainUnavailable(String),
    #[error("Network error: {0}")] NetworkError(String),
    #[error("Invalid argument: {0}")] InvalidArgument(String),
    #[error("Signing failed: {0}")] SigningFailed(String),
    /// 用户拒绝签名。来源：`KeyVaultError::UserRejected` 透传。
    #[error("User rejected signing")] UserRejected,
    #[error("Configuration error: {0}")] ConfigError(String),
    #[error("Operation not supported: {0}")] Unsupported(String),
}
```

### 4.3 `src/types.rs`

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParticipantRecord {
    pub participant_id: u32,               // 链上主键，单调递增
    pub controller_did: String,            // did:web
    pub operator: String,                  // SS58 地址
    pub chain_role: ChainRole,
    pub business_types: Vec<BusinessType>, // 语义描述，**不参与鉴权**（S20）
    pub authorized_domains: Vec<String>,   // 仅 Issuer 有意义
    pub active: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChainRole { Verifier, Issuer, ChainOwner }
// 线性包含：Verifier ⊂ Issuer ⊂ ChainOwner

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BusinessType {
    ChainOwner, LegalAnchor, CarrierWitness, AppMembership,
    EmailBinding, DomainControl, EmailGateway, RelyingParty,
    Warrant, Delegation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnchorRecord {
    pub anchor_id: String,
    pub anchor_type: AnchorType,
    pub submitter_id: u32,                 // 提交者（治理审计）
    pub issuer_did: String,                // 语义签发者（信任链根，S16）
    pub delegated_by_did: Option<String>,  // 委托锚定的代理方 DID
    pub payload_hash: String,
    pub revoked: bool,
    pub created_at: u64,
    pub valid_until: u64,                  // 0 = 永久
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DidRecord {
    pub did: String,
    pub doc_hash: Option<String>,          // 链上可选
    pub owner: Option<String>,
    pub block_number: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DidDocument {
    pub id: String,
    pub verification_methods: Vec<VerificationMethod>,
    // ... 其余 W3C DID Document 字段按 did-sdk 的实际类型
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationMethod {
    pub id: String,
    pub controller: String,
    pub public_key_bytes: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(tag = "kind", content = "name")]
pub enum AnchorType {
    EmailBinding, DomainControl, DidDocument, Warrant,
    Delegation, OrgMembership, IssuedVc, GatewayAccess,
    Custom(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AssertionPurpose {
    ParticipantRegistration,
    VcIssuance,
    DelegationAck,
}

#[derive(Debug, Clone)]
pub struct Ed25519Signature(pub Vec<u8>);

#[derive(Debug, Clone)]
pub struct Ed25519PublicKey(pub [u8; 32]);

#[derive(Debug, Clone)]
pub struct Sr25519Signature(pub Vec<u8>);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthStatus {
    pub healthy: bool,
    pub mode: String,
    pub detail: Option<String>,
    pub block_number: Option<u64>,
}
```

### 4.4 `src/events.rs`

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventData {
    pub event_name: String,
    pub anchor_id: Option<String>,
    pub participant_id: Option<u32>,
    pub block_number: Option<u64>,
    pub payload: serde_json::Value,
}

pub type Unsubscribe = Box<dyn FnOnce() + Send + Sync>;
```

### 4.5 `src/role_gate.rs`

```rust
use crate::{ChainSdkError, SdkRole};

pub(crate) trait RoleGate {
    fn role(&self) -> SdkRole;
    fn self_participant_id(&self) -> u32;

    /// 写方法进入后端逻辑之前必须调用。Issuer 放行；Verifier 返回 Unauthorized。
    fn ensure_issuer(&self) -> Result<(), ChainSdkError> {
        match self.role() {
            SdkRole::Issuer => Ok(()),
            SdkRole::Verifier => Err(ChainSdkError::Unauthorized(
                "role Verifier cannot perform write operation".into())),
        }
    }

    /// 参与方自身管理写（`update_authorized_domains` / `update_controller_did`）：
    /// 角色放行后，**仅允许修改自身 participant_id**（S19）。越权返回 Unauthorized。
    fn ensure_self(&self, participant_id: u32) -> Result<(), ChainSdkError> {
        self.ensure_issuer()?;
        if participant_id != self.self_participant_id() {
            return Err(ChainSdkError::Unauthorized(format!(
                "participant {} cannot manage participant {}; only chain-owner-sdk can",
                self.self_participant_id(), participant_id)));
        }
        Ok(())
    }

    /// 断言签名：仅 Issuer 且加载了 `participant-did/` 密钥。Verifier 返回 SignerUnavailable。
    fn ensure_signer(&self) -> Result<(), ChainSdkError> {
        match self.role() {
            SdkRole::Issuer => Ok(()),
            SdkRole::Verifier => Err(ChainSdkError::SignerUnavailable(
                "role Verifier has no participant-did signer".into())),
        }
    }
}
```

**关键约定**：
- 所有写方法（`create_anchor` / `revoke_anchor` / `renew_anchor` /
  `update_authorized_domains` / `update_controller_did`）**进入后端逻辑之前**必须先
  `self.ensure_issuer()?`（`create_anchor` 等）或 `self.ensure_self(id)?`
  （`update_*`）。
- `sign_participant_assertion` **进入后端逻辑之前**必须先 `self.ensure_signer()?`。

### 4.6 `src/keys.rs`（两类密钥 namespace 管理，S14）

```rust
use std::sync::Arc;
use keyvault_core::{KeyId, KeyVault, KeyVaultError};
use crate::{ChainSdkError, types::{AssertionPurpose, Ed25519Signature, Sr25519Signature}};

pub(crate) const CHAIN_ACCOUNT_PREFIX: &str = "chain-account/";
pub(crate) const PARTICIPANT_DID_PREFIX: &str = "participant-did/";

#[derive(Clone)]
pub(crate) struct LoadedKeys {
    pub chain_account: KeyId,       // 必以 chain-account/ 开头
    pub participant_did: KeyId,     // 必以 participant-did/ 开头
}

pub(crate) fn load_keys(vault: &dyn KeyVault, keys: &crate::config::KeyVaultKeys)
    -> Result<LoadedKeys, ChainSdkError>
{
    if !keys.operator.starts_with(CHAIN_ACCOUNT_PREFIX) {
        return Err(ChainSdkError::KeyNamespaceViolation(format!(
            "operator key must start with {}; got {}",
            CHAIN_ACCOUNT_PREFIX, keys.operator)));
    }
    if !keys.participant_did.starts_with(PARTICIPANT_DID_PREFIX) {
        return Err(ChainSdkError::KeyNamespaceViolation(format!(
            "participant_did key must start with {}; got {}",
            PARTICIPANT_DID_PREFIX, keys.participant_did)));
    }
    // 注意：这里不做实际的 vault.load（因为 load 是 async），由 impl 在写方法里按需调用
    Ok(LoadedKeys {
        chain_account: KeyId(keys.operator.clone()),
        participant_did: KeyId(keys.participant_did.clone()),
    })
}

/// **唯一入口**：sr25519 签名（extrinsic / HTTP 写请求体）。
/// 强制 namespace 校验（S14）：拒绝 `participant-did/`。
pub(crate) async fn sign_extrinsic(
    vault: &dyn KeyVault,
    key: &KeyId,
    call: &[u8],
) -> Result<Sr25519Signature, ChainSdkError> {
    if !key.0.starts_with(CHAIN_ACCOUNT_PREFIX) {
        return Err(ChainSdkError::KeyNamespaceViolation(format!(
            "sign_extrinsic requires {} namespace; got {}",
            CHAIN_ACCOUNT_PREFIX, key.0)));
    }
    let sig = vault.sign(key, call).await.map_err(crate::map_vault_err)?;
    Ok(Sr25519Signature(sig.0))
}

/// **唯一入口**：ed25519 参与方断言签名。
/// 强制 namespace 校验（S14）：拒绝 `chain-account/`。
pub(crate) async fn sign_participant_assertion_inner(
    vault: &dyn KeyVault,
    key: &KeyId,
    payload: &[u8],
    _purpose: AssertionPurpose,
) -> Result<Ed25519Signature, ChainSdkError> {
    if !key.0.starts_with(PARTICIPANT_DID_PREFIX) {
        return Err(ChainSdkError::KeyNamespaceViolation(format!(
            "sign_participant_assertion requires {} namespace; got {}",
            PARTICIPANT_DID_PREFIX, key.0)));
    }
    let sig = vault.sign(key, payload).await.map_err(crate::map_vault_err)?;
    Ok(Ed25519Signature(sig.0))
}
```

**设计说明**：`sign_extrinsic` 与 `sign_participant_assertion_inner` 是**两个独立函数**，
类型级不可互换（返回 `Sr25519Signature` vs `Ed25519Signature`）；前缀校验在函数入口
强制。`participant-did/` 与 `chain-account/` 的物理目录由 `keyvault-core` 的
`index.json` 决定；`chain-sdk` 层逻辑隔离足够满足 S14。

### 4.7 `src/did_adapter.rs`（按角色装配 did-sdk，S17）

```rust
use std::sync::Arc;
use crate::{ChainSdkError, config::ChainSdkConfig, SdkRole};

/// 按角色装配 did-sdk 能力子集。**不做 Client / Server 模式的传播**——`chain-sdk`
/// 内部实例化 Server 模式，feature 只控制能力加减。
pub(crate) fn build_did_sdk(
    cfg: &ChainSdkConfig,
    role: SdkRole,
) -> Result<Arc<dyn did_sdk::DidSdk>, ChainSdkError> {
    let mut overrides = did_sdk::config::ConfigOverrides::default();
    // did-resolution：两角色都需要
    // vc-wallet：按 cfg.capabilities.vc_wallet
    // issuer-signing：仅 Issuer
    // 具体字段名以 did-sdk 实际 ConfigOverrides 为准
    overrides.mode = Some(did_sdk_mode_for(role));
    let did_cfg = did_sdk::config::resolve_config(overrides)
        .map_err(|e| ChainSdkError::ConfigError(format!("did-sdk: {e}")))?;
    did_sdk::build_did_sdk(&did_cfg)
        .map_err(|e| ChainSdkError::ConfigError(format!("did-sdk: {e}")))
}

fn did_sdk_mode_for(role: SdkRole) -> String {
    // did-sdk 的 Server 模式（服务端参与方）；具体取值以 did-sdk 最新规格为准
    match role {
        SdkRole::Verifier => "centralized".into(),  // 只读路径
        SdkRole::Issuer => "centralized".into(),    // 签发路径
    }
}
```

> **说明**：`did-sdk` 的 `ConfigOverrides` / `Capability` 具体字段以 `did-sdk` 最新
> 规格书为准。`chain-sdk` 只消费 `DidSdk` trait 的 `resolve_did_key` /
> `resolve_did_web` / `resolve_did` / `assemble_vc_with_proof`（后者由 `did-sdk`
> 内部调用 `sign_participant_assertion` 产生的 `Proof` 组装 VC——**`chain-sdk` 不提供
> VC 存储接口**）。

### 4.8 `src/config.rs`

```rust
use std::path::PathBuf;
use std::sync::Arc;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use crate::ChainSdkError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChainSdkMode { Mock, Centralized, Chain }

/// **两角色线性包含**：`Verifier ⊂ Issuer`。
/// `role = "app"` 向后兼容映射为 `Issuer`（原 App = Verifier + Issuer ≈ Issuer）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SdkRole {
    Verifier,
    #[serde(alias = "app")]
    Issuer,
}

impl SdkRole {
    /// 是否需要加载 `KeyVault`（Issuer 才需要）。
    pub fn needs_keyvault(self) -> bool {
        matches!(self, SdkRole::Issuer)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyVaultKeys {
    pub operator: String,            // 必以 "chain-account/" 开头
    pub participant_did: String,     // 必以 "participant-did/" 开头
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyVaultConfig {
    pub scheme: String,
    pub path: PathBuf,
    pub password_env: String,
    pub password_file: PathBuf,
    pub allow_auto_generate_password: bool,
    pub keys: KeyVaultKeys,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParticipantConfig {
    pub controller_did: String,      // did:web
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capabilities {
    pub did_resolution: bool,
    pub vc_wallet: bool,
    pub issuer_signing: bool,
}

impl Default for Capabilities {
    fn default() -> Self {
        Self { did_resolution: true, vc_wallet: false, issuer_signing: false }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainSdkConfig {
    pub mode: ChainSdkMode,
    pub role: SdkRole,
    pub participant_id: u32,
    pub key_vault: KeyVaultConfig,
    pub participant: ParticipantConfig,
    pub capabilities: Capabilities,
    pub centralized_endpoint: String,
    pub chain_endpoint: String,
    pub require_finalized_node: bool,
    pub cache_ttl_secs: u64,
    pub valid_until_refresh_window_secs: u64,
    #[serde(skip)]
    pub resolved_password: Option<SecretString>,
}

#[derive(Debug, Default, Deserialize)]
pub struct ConfigOverrides {
    pub mode: Option<String>,
    pub role: Option<String>,
    pub participant_id: Option<u32>,
    pub controller_did: Option<String>,
    pub key_vault_scheme: Option<String>,
    pub key_vault_path: Option<PathBuf>,
    pub key_vault_password: Option<String>,
    pub key_vault_password_file: Option<PathBuf>,
    pub operator_key_id: Option<String>,
    pub participant_did_key_id: Option<String>,
    pub vc_wallet: Option<bool>,
    pub issuer_signing: Option<bool>,
    pub centralized_endpoint: Option<String>,
    pub chain_endpoint: Option<String>,
    pub require_finalized_node: Option<bool>,
    pub cache_ttl_secs: Option<u64>,
}

// Default / resolve_config / apply_* 逻辑按 v1 的模式实现；关键差异：
// - `participant_id` 替换 `issuer_id`
// - `key_vault.keys` / `participant.controller_did` / `capabilities` 三段
// - 仅 `role.needs_keyvault()` 时解析密码
// - `apply_overrides` 中若 role 改变，同步重算 `issuer_signing`（Verifier 强制 false）

pub fn build_chain_sdk(cfg: &ChainSdkConfig) -> Result<Arc<dyn crate::ChainSdk>, ChainSdkError> {
    // 1. 装配 KeyVault（仅 Issuer）
    let vault: Option<Arc<dyn keyvault_core::KeyVault>> = match cfg.role {
        SdkRole::Issuer => Some(build_vault(cfg)?),
        SdkRole::Verifier => None,
    };
    // 2. 装配 did-sdk（两角色都装配 did-resolution；issuer-signing 仅 Issuer）
    let did = Some(crate::did_adapter::build_did_sdk(cfg, cfg.role)?);
    // 3. 加载两类密钥的 KeyId（仅 Issuer）
    let keys = match cfg.role {
        SdkRole::Issuer => {
            let v = vault.as_ref().expect("issuer requires vault");
            Some(crate::keys::load_keys(v.as_ref(), &cfg.key_vault.keys)?)
        }
        SdkRole::Verifier => None,
    };
    let core = crate::SdkCore {
        vault, did, config: cfg.clone(), role: cfg.role, keys,
    };
    match cfg.mode {
        ChainSdkMode::Mock => Ok(Arc::new(crate::mock::MockChainSdk::new(core))),
        ChainSdkMode::Centralized => Ok(Arc::new(crate::centralized::CentralizedChainSdk::new(core)?)),
        ChainSdkMode::Chain => Ok(Arc::new(crate::chain::ChainModeSdk::new(core)?)),
    }
}

// build_vault / resolve_password：按 v1 的模式实现，scheme 分支同 v1
```

### 4.9 `src/mock.rs`（核心逻辑）

```rust
use std::collections::{HashMap, HashSet};
use async_lock::RwLock;
use async_trait::async_trait;
use crate::{
    AnchorRecord, AnchorType, AssertionPurpose, ChainSdk, ChainSdkError,
    DidRecord, Ed25519PublicKey, Ed25519Signature, EventData, HealthStatus,
    ParticipantRecord, SdkCore, SdkRole, Unsubscribe,
};
use crate::role_gate::RoleGate;

pub struct MockState {
    pub anchors: HashMap<String, AnchorRecord>,
    pub participants: HashMap<u32, ParticipantRecord>,
    pub did_participant: HashMap<String, u32>,          // DidParticipant 索引
    pub account_participant: HashMap<String, u32>,      // AccountParticipant 索引
    pub registered_anchor_types: HashSet<AnchorType>,
    pub dids: HashMap<String, DidRecord>,
    pub current_time: u64,
    pub next_anchor_seq: u64,
}

pub enum FailureRule {
    Latency { op: String, millis: u64 },
    Unavailable,
    Specific { op: String, error: String },
}

pub struct MockChainSdk {
    pub(crate) core: SdkCore,
    pub(crate) state: RwLock<MockState>,
    pub(crate) subscribers: std::sync::RwLock<Vec<(u64, String, std::sync::Arc<dyn Fn(EventData) + Send + Sync>)>>,
    pub(crate) next_sub_id: std::sync::atomic::AtomicU64,
    pub(crate) failures: RwLock<Vec<FailureRule>>,
    pub(crate) unavailable: std::sync::atomic::AtomicBool,
    pub(crate) persistence_path: Option<std::path::PathBuf>,
}

// 方法实现要点（按 v1 的模式扩展）：
// - create_anchor: ensure_issuer → 生成 anchor_id → 写入
//   AnchorRecord { submitter_id = self.participant_id, issuer_did = self.controller_did,
//                  delegated_by_did = None, ... }
// - get_participant_by_did: 先查 state.did_participant，再取 participants
// - sign_participant_assertion: ensure_signer → keys.participant_did →
//   crate::keys::sign_participant_assertion_inner
// - update_authorized_domains / update_controller_did: ensure_self(participant_id) →
//   更新自身参与者记录 → persist
// - resolve_did_key: 调 core.did 的 resolve_did_key（同步）
// - resolve_did_web: 调 core.did 的 resolve_did_web（异步）
// - resolve_did: 查 state.dids
// - 其它方法同 v1 模式

impl RoleGate for MockChainSdk {
    fn role(&self) -> SdkRole { self.core.role }
    fn self_participant_id(&self) -> u32 { self.core.config.participant_id }
}

#[async_trait]
impl ChainSdk for MockChainSdk {
    // 读方法（省略骨架，按 v1 模式 + state 新字段实现）
    async fn health_check(&self) -> Result<HealthStatus, ChainSdkError> { /* ... */ }
    async fn get_anchor(&self, id: &str) -> Result<AnchorRecord, ChainSdkError> { /* ... */ }
    async fn list_anchors(&self, t: Option<AnchorType>, s: Option<u32>)
        -> Result<Vec<AnchorRecord>, ChainSdkError>
    {
        let st = self.state.read().await;
        Ok(st.anchors.values()
            .filter(|a| t.as_ref().map_or(true, |x| &a.anchor_type == x))
            .filter(|a| s.map_or(true, |x| a.submitter_id == x))
            .cloned().collect())
    }
    async fn get_participant(&self, id: u32) -> Result<ParticipantRecord, ChainSdkError>
    { self.state.read().await.participants.get(&id).cloned()
        .ok_or(ChainSdkError::ParticipantNotFound(id)) }
    async fn get_participant_by_did(&self, did: &str)
        -> Result<Option<ParticipantRecord>, ChainSdkError>
    {
        let st = self.state.read().await;
        Ok(st.did_participant.get(did).and_then(|id| st.participants.get(id).cloned()))
    }
    async fn list_participants(&self) -> Result<Vec<ParticipantRecord>, ChainSdkError>
    { Ok(self.state.read().await.participants.values().cloned().collect()) }

    async fn resolve_did(&self, did: &str) -> Result<Option<DidRecord>, ChainSdkError>
    { Ok(self.state.read().await.dids.get(did).cloned()) }

    fn resolve_did_key(&self, did: &str) -> Result<Ed25519PublicKey, ChainSdkError> {
        // 委托 did-sdk；若 core.did 为 None 返回 SignerUnavailable
        let did = self.core.did.as_ref()
            .ok_or_else(|| ChainSdkError::SignerUnavailable("did-sdk not assembled".into()))?;
        did.resolve_did_key(did).map(Ed25519PublicKey)
            .map_err(|e| ChainSdkError::InvalidArgument(format!("did-sdk: {e}")))
    }
    async fn resolve_did_web(&self, did: &str) -> Result<crate::types::DidDocument, ChainSdkError> {
        let d = self.core.did.as_ref()
            .ok_or_else(|| ChainSdkError::SignerUnavailable("did-sdk not assembled".into()))?;
        d.resolve_did_web(did).await
            .map_err(|e| ChainSdkError::InvalidArgument(format!("did-sdk: {e}")))
    }

    async fn create_anchor(&self, t: AnchorType, h: &str, v: u64)
        -> Result<String, ChainSdkError>
    {
        self.ensure_issuer()?;
        let _ = self.core.vault.as_ref()
            .ok_or_else(|| ChainSdkError::SignerUnavailable("issuer requires KeyVault".into()))?;
        let _ = &self.core.keys;
        let mut st = self.state.write().await;
        if !st.registered_anchor_types.contains(&t) {
            return Err(ChainSdkError::AnchorTypeNotRegistered(t));
        }
        st.next_anchor_seq += 1;
        let id = format!("anchor:{}", st.next_anchor_seq);
        st.anchors.insert(id.clone(), AnchorRecord {
            anchor_id: id.clone(), anchor_type: t,
            submitter_id: self.core.config.participant_id,
            issuer_did: self.core.config.participant.controller_did.clone(),
            delegated_by_did: None,
            payload_hash: h.into(), revoked: false,
            created_at: st.current_time, valid_until: v,
        });
        drop(st);
        self.emit_created(&id, h).await;
        self.persist().await;
        Ok(id)
    }

    async fn revoke_anchor(&self, id: &str) -> Result<(), ChainSdkError> { /* 同 v1 */ }
    async fn renew_anchor(&self, id: &str, v: u64) -> Result<(), ChainSdkError> { /* 同 v1 */ }

    async fn update_authorized_domains(&self, pid: u32, domains: Vec<String>)
        -> Result<(), ChainSdkError>
    {
        self.ensure_self(pid)?;                       // S19：仅自身
        let mut st = self.state.write().await;
        let rec = st.participants.get_mut(&pid)
            .ok_or(ChainSdkError::ParticipantNotFound(pid))?;
        rec.authorized_domains = domains;
        drop(st); self.persist().await; Ok(())
    }
    async fn update_controller_did(&self, pid: u32, new_did: &str) -> Result<(), ChainSdkError> {
        self.ensure_self(pid)?;                       // S19：仅自身
        let mut st = self.state.write().await;
        let rec = st.participants.get_mut(&pid)
            .ok_or(ChainSdkError::ParticipantNotFound(pid))?;
        // 更新双向索引：旧 DID 移除、新 DID 加入
        st.did_participant.remove(&rec.controller_did);
        st.did_participant.insert(new_did.into(), pid);
        rec.controller_did = new_did.into();
        drop(st); self.persist().await; Ok(())
    }

    async fn sign_participant_assertion(&self, payload: &[u8], purpose: AssertionPurpose)
        -> Result<Ed25519Signature, ChainSdkError>
    {
        self.ensure_signer()?;                        // S9 / S17
        let vault = self.core.vault.as_ref()
            .ok_or_else(|| ChainSdkError::SignerUnavailable("issuer requires KeyVault".into()))?;
        let keys = self.core.keys.as_ref()
            .ok_or_else(|| ChainSdkError::SignerUnavailable("keys not loaded".into()))?;
        crate::keys::sign_participant_assertion_inner(
            vault.as_ref(), &keys.participant_did, payload, purpose).await
    }

    fn subscribe_event(&self, name: &str, cb: Box<dyn Fn(EventData) + Send + Sync>)
        -> Result<Unsubscribe, ChainSdkError>
    {
        let id = self.next_sub_id.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let cb: std::sync::Arc<dyn Fn(EventData) + Send + Sync> = cb.into();
        self.subscribers.write().unwrap().push((id, name.into(), cb));
        let subs = std::sync::Arc::new(());
        let _ = subs;
        Ok(Box::new(move || { /* 从 subscribers 移除 id（实现时用 Arc<RwLock<Vec>> 共享） */ }))
    }
}

// helpers: emit_created / persist / check_failure / advance_time / set_chain_unavailable / add_failure
// 同 v1 模式
```

### 4.10 `src/centralized.rs`（与 v1 的差异）

与 v1 基本一致，关键差异：

- **写请求体签名**：使用 `crate::keys::sign_extrinsic(vault, &keys.chain_account, canonical)`
  而非直接 `vault.sign`，从而**强制 namespace 校验**（S14）。
- **断言签名**：`sign_participant_assertion` 直接复用 `keys::sign_participant_assertion_inner`，
  不走 HTTP（断言是链下协议对象，不由中心化服务代签）。
- **HTTP 端点映射**：
  - `get_participant(id)` → `GET /participants/{id}`
  - `get_participant_by_did(did)` → `GET /participants/by-did/{did}`
  - `list_participants()` → `GET /participants`
  - `update_authorized_domains` → `POST /participants/{id}/domains`
  - `update_controller_did` → `POST /participants/{id}/controller-did`
  - 不再有 `GET /issuers/{id}` / `POST /issuers/...`。
- **`list_anchors` 查询参数**：`submitter_id` 替换 `issuer_id`。
- **TTL 缓存**（S8）：仅 `get_anchor` 启用，与 v1 一致。

关键片段：

```rust
async fn signed_request<T: serde::Serialize>(
    &self, op: &str, path: &str, body: &T,
) -> Result<reqwest::Response, ChainSdkError> {
    self.ensure_issuer()?;
    let vault = self.core.vault.as_ref()
        .ok_or_else(|| ChainSdkError::SignerUnavailable("issuer requires KeyVault".into()))?;
    let keys = self.core.keys.as_ref()
        .ok_or_else(|| ChainSdkError::SignerUnavailable("keys not loaded".into()))?;
    let canonical = canonical_bytes(op, body)?;
    // S14：通过 keys::sign_extrinsic 强制 chain-account/ namespace
    let sig = crate::keys::sign_extrinsic(vault.as_ref(), &keys.chain_account, &canonical).await?;
    let url = format!("{}{}", self.core.config.centralized_endpoint, path);
    self.http.post(&url)
        .header("x-chain-signer", &keys.chain_account.0)
        .header("x-chain-signature", hex::encode(&sig.0))
        .header("x-chain-op", op)
        .json(body).send().await
        .map_err(|e| ChainSdkError::NetworkError(e.to_string()))
}

async fn sign_participant_assertion(&self, payload: &[u8], purpose: AssertionPurpose)
    -> Result<Ed25519Signature, ChainSdkError>
{
    self.ensure_signer()?;
    let vault = self.core.vault.as_ref()
        .ok_or_else(|| ChainSdkError::SignerUnavailable("issuer requires KeyVault".into()))?;
    let keys = self.core.keys.as_ref()
        .ok_or_else(|| ChainSdkError::SignerUnavailable("keys not loaded".into()))?;
    crate::keys::sign_participant_assertion_inner(
        vault.as_ref(), &keys.participant_did, payload, purpose).await
}
```

### 4.11 `src/chain.rs`（subxt 骨架，POC 占位）

与 v1 一致：接口、错误码、角色拦截、事件订阅订阅必须完整；subxt 查询与
extrinsic 提交的具体实现允许 `Err(Unsupported(...))` 占位，但：

- **所有写方法**必须先 `self.ensure_issuer()?`（`create_anchor` 等）或
  `self.ensure_self(pid)?`（`update_*`）；
- **`sign_participant_assertion`** 必须先 `self.ensure_signer()?`，然后走
  `keys::sign_participant_assertion_inner`（与 Mock/Centralized 一致，不依赖链连接）；
- **`sign_extrinsic` 不在 trait 中**——它是内部工具，由 Centralized/Chain 后端在
  写操作时调用 `keys::sign_extrinsic`。

---

## 5. 配置文件示例

### 5.1 `configs/chain-sdk.toml.example`

```toml
[chain_sdk]
mode = "chain"                 # mock / centralized / chain
role = "issuer"                # verifier / issuer（role = "app" 别名映射为 issuer）
participant_id = 4
cache_ttl_secs = 300
valid_until_refresh_window_secs = 86400

# Issuer 必填；Verifier 不填，SDK 不加载 KeyVault
[chain_sdk.key_vault]
scheme = "file-keystore"
path = "./participant-keys"
password_env = "CHAIN_KEYSTORE_PASSWORD"
password_file = "./participant-keys/password"
allow_auto_generate_password = true

[chain_sdk.key_vault.keys]
operator = "chain-account/operator-1"                          # sr25519
participant_did = "participant-did/did_web_mail_example_com"   # ed25519

[chain_sdk.participant]
controller_did = "did:web:mail.example.com:issuer"

[chain_sdk.capabilities]
did_resolution = true
vc_wallet = false
issuer_signing = true          # 仅 Issuer；Verifier 强制 false

[chain_sdk.centralized]
endpoint = "http://localhost:8080"

[chain_sdk.chain]
endpoint = "wss://rpc.identity-chain.com:9944"
require_finalized_node = true
```

### 5.2 `tests/configs/verifier.toml` / `issuer.toml`

```toml
# verifier.toml
[chain_sdk]
mode = "mock"
role = "verifier"
participant_id = 0

[chain_sdk.key_vault]
scheme = "memory-keystore"
path = "/tmp/unused"
password_env = "UNUSED"
password_file = "/tmp/unused"
allow_auto_generate_password = false
[chain_sdk.key_vault.keys]
operator = "chain-account/unused"
participant_did = "participant-did/unused"

[chain_sdk.participant]
controller_did = "did:web:verifier.example.com"

[chain_sdk.capabilities]
did_resolution = true
vc_wallet = false
issuer_signing = false
```

```toml
# issuer.toml
[chain_sdk]
mode = "mock"
role = "issuer"
participant_id = 4

[chain_sdk.key_vault]
scheme = "memory-keystore"
path = "/tmp/unused"
password_env = "UNUSED"
password_file = "/tmp/unused"
allow_auto_generate_password = false
[chain_sdk.key_vault.keys]
operator = "chain-account/operator-1"
participant_did = "participant-did/did_web_mail_example_com"

[chain_sdk.participant]
controller_did = "did:web:mail.example.com:issuer"

[chain_sdk.capabilities]
did_resolution = true
vc_wallet = false
issuer_signing = true
```

### 5.3 环境变量覆盖表

| 环境变量 | 覆盖项 |
|---|---|
| `CHAIN_SDK_HOME` / `CHAIN_SDK_CONFIG_FILE` | 根目录 / 配置路径 |
| `CHAIN_SDK_MODE` | `mock` / `centralized` / `chain` |
| `CHAIN_SDK_ROLE` | `verifier` / `issuer`（`app` 为 alias） |
| `CHAIN_SDK_PARTICIPANT_ID` | `participant_id` |
| `CHAIN_SDK_CONTROLLER_DID` | `participant.controller_did` |
| `CHAIN_KEYSTORE_PASSWORD` | 密钥库密码（仅 Issuer） |
| `CHAIN_SDK_KEYVAULT_SCHEME` / `_PATH` / `_PASSWORD_FILE` | KeyVault 配置 |
| `CHAIN_SDK_OPERATOR_KEY_ID` / `_PARTICIPANT_DID_KEY_ID` | 两类密钥的 KeyId |
| `CHAIN_SDK_VC_WALLET` / `_ISSUER_SIGNING` | 能力开关 |
| `CHAIN_SDK_CENTRALIZED_ENDPOINT` / `_CHAIN_ENDPOINT` | 端点 |
| `CHAIN_SDK_REQUIRE_FINALIZED` / `_CACHE_TTL_SECS` | 行为 |

**优先级**：显式 `ConfigOverrides` > 环境变量 > TOML 文件 > 内置默认值。
密码单独一条链：`CHAIN_KEYSTORE_PASSWORD` → `password_file` →
`allow_auto_generate_password` 自动生成 → 报错。仅 `role = Issuer` 时解析。

---

## 6. 测试方案（覆盖 S1–S20）

| 测试文件 | 覆盖 | 说明 |
|---|---|---|
| `tests/config_resolution.rs` | S2 | 默认值 / env / override 优先；`verifier` 不解析密码；`role = "app"` 映射为 Issuer |
| `tests/role_matrix.rs` | S2 / S9 | 3 后端 × 2 角色；`chain` 后端 `Issuer` 写操作返回 `Unsupported`（非 `Unauthorized`） |
| `tests/role_gate.rs` | S9 / S19 | 见 §6.1 |
| `tests/capabilities.rs` | S17 | Verifier：`core.did.is_some()`、`core.vault.is_none()`；Issuer：二者均 `Some`；`core.keys` 仅 Issuer 有 |
| `tests/keys_namespace.rs` | S14 | 见 §6.2 |
| `tests/participant_binding.rs` | S15 / S16 / S20 | 见 §6.3 |
| `tests/no_governance.rs` | S18 | 见 §6.4 |
| `tests/consistency.rs` | S3 / S12 | 同一组用例在 Mock 与 Centralized（wiremock）上结果一致 |
| `tests/mock_persistence.rs` | S4 | 预置后读得到；写后 JSON 更新 |
| `tests/mock_fault_injection.rs` | S5 | 延迟 / 不可用 / 指定错误 |
| `tests/centralized_contract.rs` | S6 | wiremock 断言 method + path（含新端点） |
| `tests/centralized_signing.rs` | S7 | `x-chain-signer` / `x-chain-signature` / `x-chain-op` |
| `tests/centralized_cache.rs` | S8 | TTL 命中 + 过期穿透 |
| `tests/keyvault_integration.rs` | S10 / S13 | Issuer 加载 FileKeystore；Verifier 不加载 |
| `tests/event_subscription.rs` | S11 | 三后端均能收到事件 |

### 6.1 `tests/role_gate.rs`（S9 + S19）

```rust
#[tokio::test]
async fn verifier_cannot_write_any_backend() {
    for mode in ["mock", "centralized"] {
        let sdk = build_issuer_or_verifier(mode, "verifier");
        for r in [
            sdk.create_anchor(AnchorType::EmailBinding, "h", 0).await.map(|_| ()),
            sdk.revoke_anchor("a").await,
            sdk.renew_anchor("a", 1).await,
            sdk.update_authorized_domains(0, vec![]).await,
            sdk.update_controller_did(0, "x").await,
        ] {
            assert!(matches!(r, Err(ChainSdkError::Unauthorized(_))));
        }
        // 断言签名：Verifier → SignerUnavailable
        let r = sdk.sign_participant_assertion(b"msg", AssertionPurpose::VcIssuance).await;
        assert!(matches!(r, Err(ChainSdkError::SignerUnavailable(_))));
    }
}

#[tokio::test]
async fn issuer_cannot_update_other_participants() {
    let sdk = build_issuer("mock");   // participant_id = 4
    // 自身：允许（Mock 中状态可预置 participant 4）
    let r = sdk.update_authorized_domains(4, vec!["a.com".into()]).await;
    assert!(r.is_ok() || matches!(r, Err(ChainSdkError::ParticipantNotFound(_))));
    // 他人：Unauthorized
    let r = sdk.update_authorized_domains(999, vec![]).await;
    assert!(matches!(r, Err(ChainSdkError::Unauthorized(_))));
    let r = sdk.update_controller_did(999, "did:web:evil").await;
    assert!(matches!(r, Err(ChainSdkError::Unauthorized(_))));
}
```

### 6.2 `tests/keys_namespace.rs`（S14）

```rust
#[tokio::test]
async fn sign_extrinsic_rejects_participant_did_namespace() {
    let vault = keyvault_core::MemoryKeystore::new();
    // 直接调用内部函数：sign_extrinsic 必须以 "chain-account/" 开头
    let bad = keyvault_core::KeyId("participant-did/foo".into());
    let r = chain_sdk::keys::sign_extrinsic(&vault, &bad, b"payload").await;
    assert!(matches!(r, Err(ChainSdkError::KeyNamespaceViolation(_))));
}

#[tokio::test]
async fn sign_participant_assertion_rejects_chain_account_namespace() {
    let vault = keyvault_core::MemoryKeystore::new();
    let bad = keyvault_core::KeyId("chain-account/operator-1".into());
    let r = chain_sdk::keys::sign_participant_assertion_inner(
        &vault, &bad, b"payload", AssertionPurpose::VcIssuance).await;
    assert!(matches!(r, Err(ChainSdkError::KeyNamespaceViolation(_))));
}

#[test]
fn load_keys_rejects_wrong_prefix() {
    // keys::load_keys 在构造时就校验前缀
    // 通过 resolve_config + build_chain_sdk 触发
    let cfg = resolve_config(ConfigOverrides {
        mode: Some("mock".into()), role: Some("issuer".into()),
        operator_key_id: Some("participant-did/wrong".into()),
        ..Default::default()
    }).unwrap();
    let r = build_chain_sdk(&cfg);
    assert!(matches!(r, Err(ChainSdkError::KeyNamespaceViolation(_))));
}
```

### 6.3 `tests/participant_binding.rs`（S15 / S16 / S20）

```rust
#[tokio::test]
async fn register_participant_writes_bidirectional_index() {
    // Mock 预置包含双向索引的 ParticipantRecord（见 fixtures）
    let sdk = build_verifier("mock");
    let p = sdk.get_participant(4).await.unwrap();
    assert_eq!(p.controller_did, "did:web:mail.example.com:issuer");
    assert_eq!(p.operator, "5GrwvaEF...");   // SS58
    assert_eq!(p.chain_role, ChainRole::Issuer);
    let by_did = sdk.get_participant_by_did(&p.controller_did).await.unwrap();
    assert!(by_did.is_some());
    assert_eq!(by_did.unwrap().participant_id, 4);
}

#[tokio::test]
async fn delegated_anchor_keeps_issuer_did_as_trust_root() {
    // fixtures/delegated_anchor.json 描述一条委托锚定：
    // issuer_did = A_did, submitter_id = S_participant_id, delegated_by_did = S_did
    let sdk = build_verifier("mock");
    let anchor = sdk.get_anchor("anchor:delegated-1").await.unwrap();
    assert_eq!(anchor.issuer_did, "did:key:zA");     // 语义签发者 = 信任根
    assert_eq!(anchor.submitter_id, 7);              // 代理方
    assert_eq!(anchor.delegated_by_did.as_deref(), Some("did:web:proxy.example.com"));
}

#[tokio::test]
async fn business_types_do_not_affect_authorization() {
    // 篡改 business_types 不影响鉴权（S20）：预置两个参与者，仅 chain_role 不同
    let sdk = build_issuer("mock");
    // 修改自身 domains：仅需 chain_role = Issuer，与 business_types 无关
    let r = sdk.update_authorized_domains(4, vec!["a.com".into()]).await;
    // business_types 不含 DomainControl，但 chain_role = Issuer → 允许
    assert!(r.is_ok() || matches!(r, Err(ChainSdkError::ParticipantNotFound(_))));
}
```

### 6.4 `tests/no_governance.rs`（S18）

```rust
#[test]
fn chain_sdk_trait_has_no_governance_methods() {
    // 通过编译期断言 / 字符串检查 / trait 对象方法表：
    // 允许的方法名（trait 定义）：
    //   health_check, get_anchor, list_anchors,
    //   get_participant, get_participant_by_did, list_participants,
    //   resolve_did, resolve_did_key, resolve_did_web,
    //   create_anchor, revoke_anchor, renew_anchor,
    //   update_authorized_domains, update_controller_did,
    //   sign_participant_assertion, subscribe_event
    // 禁止出现（应属 chain-owner-sdk）：
    //   register_participant, deactivate_participant, update_operator,
    //   register_anchor_type, submit_delegated_anchor
    //
    // 用 trybuild / 字符串扫描源码实现；最低要求：在 CI 中 grep 源码
    // `cargo check` 通过即保证 trait 实现完整；如需严格校验，可写
    // `const _: () = assert!(!has_method("register_participant"));`
}
```

> **实现建议**：在 `src/lib.rs` 顶部加注释块列出**允许的方法名**；在 CI 的
> `roles.yml` 中加一步 `grep -rE "register_participant|submit_delegated_anchor|..." src/`
> 断言无匹配。

### 6.5 `tests/fixtures/mock_state.json`（含参与方与委托锚定）

```json
{
  "anchors": {
    "anchor:1": {
      "anchor_id": "anchor:1",
      "anchor_type": {"kind": "EmailBinding"},
      "submitter_id": 4,
      "issuer_did": "did:web:mail.example.com:issuer",
      "delegated_by_did": null,
      "payload_hash": "blake2b:deadbeef",
      "revoked": false, "created_at": 0, "valid_until": 0
    },
    "anchor:delegated-1": {
      "anchor_id": "anchor:delegated-1",
      "anchor_type": {"kind": "IssuedVc"},
      "submitter_id": 7,
      "issuer_did": "did:key:zA",
      "delegated_by_did": "did:web:proxy.example.com",
      "payload_hash": "blake2b:cafe",
      "revoked": false, "created_at": 0, "valid_until": 0
    }
  },
  "participants": {
    "4": {
      "participant_id": 4,
      "controller_did": "did:web:mail.example.com:issuer",
      "operator": "5GrwvaEF...",
      "chain_role": "Issuer",
      "business_types": ["EmailBinding", "DomainControl"],
      "authorized_domains": ["mail.example.com"],
      "active": true
    },
    "7": {
      "participant_id": 7,
      "controller_did": "did:web:proxy.example.com",
      "operator": "5FHneW...",
      "chain_role": "Issuer",
      "business_types": ["Delegation"],
      "authorized_domains": [],
      "active": true
    }
  },
  "did_participant": {
    "did:web:mail.example.com:issuer": 4,
    "did:web:proxy.example.com": 7
  },
  "account_participant": {
    "5GrwvaEF...": 4,
    "5FHneW...": 7
  },
  "registered_anchor_types": [
    {"kind": "EmailBinding"}, {"kind": "DomainControl"}, {"kind": "IssuedVc"}
  ],
  "dids": {},
  "current_time": 1000,
  "next_anchor_seq": 1
}
```

---

## 7. CI 与脚本

### 7.1 `.github/workflows/ci.yml`

```yaml
name: ci
on: [push, pull_request]
jobs:
  rust:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@1.85.0
        with: { components: rustfmt, clippy }
      - run: cargo fmt --all -- --check
      - run: cargo clippy --all-targets -- -D warnings
      - run: cargo test
```

### 7.2 `.github/workflows/rust-musl.yml`

与 v1 相同（四平台矩阵；musl 用 zigbuild；`file`/`ldd` 断言 static）。

### 7.3 `.github/workflows/consistency.yml` / `roles.yml`

```yaml
# consistency.yml
name: consistency
on: [push, pull_request]
jobs:
  suite:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@1.85.0
      - run: cargo test --test consistency --test role_gate -- --nocapture
```

```yaml
# roles.yml
name: roles
on: [push, pull_request]
jobs:
  matrix:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@1.85.0
      - run: cargo test --test role_matrix --test role_gate --test keyvault_integration --test capabilities --test keys_namespace --test participant_binding
      - name: Assert no governance methods in trait
        run: |
          ! grep -rE 'fn (register_participant|deactivate_participant|update_operator|register_anchor_type|submit_delegated_anchor)\b' src/
```

### 7.4 `scripts/build-rust-musl.sh` / `run-consistency.sh` / `verify-roles.sh`

与 v1 相同；`verify-roles.sh` 增加：

```bash
cargo test --test capabilities --test keys_namespace --test participant_binding --test no_governance
```

---

## 8. 验收清单映射（S1–S20）

| 编号 | 验收项 | 落点 | 验证命令 |
|---|---|---|---|
| S1 | 接口契约完整 | `src/lib.rs` 的 `ChainSdk` trait | `cargo check` |
| S2 | 三后端模式切换 | `config.rs` + `tests/config_resolution.rs` + `tests/role_matrix.rs` | `cargo test --test config_resolution --test role_matrix` |
| S3 | 三后端语义一致 | `tests/consistency.rs` | `cargo test --test consistency` |
| S4 | Mock 状态预置与持久化 | `mock.rs::with_preloaded_state` + `tests/mock_persistence.rs` | `cargo test --test mock_persistence` |
| S5 | Mock 故障注入 | `mock.rs::add_failure` / `set_chain_unavailable` + `tests/mock_fault_injection.rs` | `cargo test --test mock_fault_injection` |
| S6 | Centralized 接口映射 | `centralized.rs` + `tests/centralized_contract.rs` | `cargo test --test centralized_contract` |
| S7 | Centralized 写签名 | `centralized.rs::signed_request` + `tests/centralized_signing.rs` | `cargo test --test centralized_signing` |
| S8 | Centralized 缓存 | `tests/centralized_cache.rs` | `cargo test --test centralized_cache` |
| S9 | Chain 角色拦截 | `role_gate.rs` + `tests/role_gate.rs` | `cargo test --test role_gate` |
| S10 | 私钥管理内置 | `config.rs::build_vault` + `tests/keyvault_integration.rs` | `cargo test --test keyvault_integration` |
| S11 | 事件订阅 | `events.rs` + 三后端 + `tests/event_subscription.rs` | `cargo test --test event_subscription` |
| S12 | 一致性测试 | `tests/consistency.rs` | `cargo test --test consistency` |
| S13 | `keyvault-core` 集成 | 外部依赖 `did-sdk/keyvault-core`；`tests/keyvault_integration.rs` | 同上 |
| **S14** | **namespace 隔离** | `keys.rs::sign_extrinsic` / `sign_participant_assertion_inner` + `tests/keys_namespace.rs` | `cargo test --test keys_namespace` |
| **S15** | **参与方身份绑定** | `types::ParticipantRecord` + `mock.rs` 双向索引 + `tests/participant_binding.rs` | `cargo test --test participant_binding` |
| **S16** | **委托锚定语义** | `types::AnchorRecord.issuer_did` / `delegated_by_did` + `tests/participant_binding.rs` | 同上 |
| **S17** | **按角色装配** | `config.rs::build_chain_sdk` + `did_adapter.rs` + `tests/capabilities.rs` | `cargo test --test capabilities` |
| **S18** | **无治理接口** | `src/lib.rs` trait 方法名 + `tests/no_governance.rs` + CI grep | `cargo test --test no_governance` |
| **S19** | **自身管理写越权拦截** | `role_gate.rs::ensure_self` + `tests/role_gate.rs` | `cargo test --test role_gate` |
| **S20** | **business_types 不参与鉴权** | `role_gate.rs` 不读取 business_types；`tests/participant_binding.rs` | `cargo test --test participant_binding` |
| **S1-static** | musl 静态链接 | `examples/mini_cli.rs` + `scripts/build-rust-musl.sh` | `bash scripts/build-rust-musl.sh` |
| **S2-UserRejected** | `UserRejected` 透传 | `map_vault_err` + 可选测试 | `cargo test --test user_rejected`（可选） |
| **S3-AppAlias** | `role = "app"` 向后兼容 | `SdkRole` serde `alias = "app"` | `cargo test --test config_resolution` |
| **K1–K9** | `keyvault-core` 验收项 | **由 `did-sdk` 仓库保证**；本仓库不重复验收 | 见 `did-sdk` 文档 |

---

## 9. 分阶段落地与完成标准

| 阶段 | 内容 | 完成标准 |
|---|---|---|
| **P0** | 工作区 + `keyvault-core` / `did-sdk` 外部依赖接入 + `types.rs` + `error.rs` + `config.rs`（含 `SdkRole` 两值 + `alias = "app"` + `Capabilities` + `KeyVaultKeys`）+ `role_gate.rs` + `keys.rs` | `cargo check` 通过；`keyvault-core` / `did-sdk` 从 git tag 拉取成功 |
| **P1** | `MockChainSdk` 完整实现（含 `ParticipantRecord` / `DidParticipant` / `AccountParticipant` 索引）+ `tests/mock_persistence.rs` + `tests/mock_fault_injection.rs` | 两测试通过 |
| **P2** | `role_gate` 两角色生效 + `ensure_self` + `tests/role_gate.rs` + `tests/role_matrix.rs` + `tests/keyvault_integration.rs` | S9 / S10 / S19 / S2 通过 |
| **P3** | `keys.rs` 两类密钥 + namespace 校验 + `tests/keys_namespace.rs` | **S14 通过** |
| **P4** | `did_adapter.rs` 按角色装配 + `tests/capabilities.rs` | **S17 通过** |
| **P5** | `CentralizedChainSdk`（新端点 + `sign_extrinsic`）+ `tests/centralized_contract.rs` + `tests/centralized_signing.rs` + `tests/centralized_cache.rs` | S6 / S7 / S8 通过 |
| **P6** | 参与方身份绑定 + 委托锚定 fixtures + `tests/participant_binding.rs` | **S15 / S16 / S20 通过** |
| **P7** | `tests/no_governance.rs` + CI grep | **S18 通过** |
| **P8** | 事件订阅三后端 + `tests/event_subscription.rs` | S11 通过 |
| **P9** | `tests/consistency.rs`（Mock vs Centralized 语义一致性） | S3 / S12 通过 |
| **P10** | `examples/mini_cli.rs` + `scripts/build-rust-musl.sh` + 四平台 CI 全绿 | S1-static 通过 |
| **P11** | `ChainModeSdk` 的 subxt 查询 + extrinsic 提交（POC 可选；角色拦截必须生效） | `chain` 模式 S9 / S17 / S19 通过 |

---

## 10. Agent 执行指令（放在文末，优先执行）

1. 在空目录中创建 `chain-sdk` 仓库，按 §1 生成目录树。
2. 按 §2 写入工作区根 `Cargo.toml`；**不要**创建 `keyvault-core/` 或 `did-sdk/`
   子目录。按 §0.2 引入两个外部依赖（默认 git tag）。
3. 按 §4 写入 `src/lib.rs` / `error.rs` / `types.rs` / `events.rs` /
   `role_gate.rs` / `keys.rs` / `did_adapter.rs` / `config.rs` / `mock.rs` /
   `centralized.rs` / `chain.rs`。
   - 标 `TODO(agent)` 的按注释语义补全；其余骨架代码可直接采用。
   - **`subscribers` 必须用 `std::sync::RwLock`**（`subscribe_event` 是同步方法）。
   - **`SdkRole` 必须是两值**：`Verifier` / `Issuer`，`Issuer` 带
     `#[serde(alias = "app")]`（S3-AppAlias）。
   - **`keys.rs` 的 `sign_extrinsic` / `sign_participant_assertion_inner` 必须
     强制 namespace 校验**（S14），返回 `ChainSdkError::KeyNamespaceViolation`。
   - **`build_chain_sdk` 必须按角色装配**：`Verifier` 不加载 `KeyVault`、
     不加载 `issuer-signing`；`Issuer` 加载两类密钥（S17）。
4. 按 §6 写入全部测试文件；`tests/fixtures/mock_state.json` 与
   `tests/configs/*.toml` 直接采用 §6.5 / §5.2 的内容。
5. 按 §7 写入 `.github/workflows/*.yml` 与 `scripts/*.sh`。
6. 运行并修复直到通过：
   - `cargo fmt --all -- --check`
   - `cargo clippy --all-targets -- -D warnings`
   - `cargo test`（含 `config_resolution`、`role_gate`、`role_matrix`、
     `capabilities`、`keys_namespace`、`participant_binding`、`no_governance`、
     `consistency`、`mock_persistence`、`mock_fault_injection`、
     `centralized_contract`、`centralized_signing`、`centralized_cache`、
     `keyvault_integration`、`event_subscription`）
   - `bash scripts/build-rust-musl.sh`（`file`/`ldd` 断言 static）
7. 在 `README.md` 中写明：
   - **两角色线性包含**：`Verifier ⊂ Issuer`；`role = "app"` 映射为 `Issuer`；
   - **`ChainRole`（链上角色）与 `SdkRole`（SDK 配置角色）的区别**；
   - **两类服务端密钥**（`chain-account/` sr25519 vs `participant-did/` ed25519）
     与两个签名方法（`sign_extrinsic` 内部 / `sign_participant_assertion` trait）；
   - **namespace 隔离**（S14）：违规返回 `KeyNamespaceViolation`；
   - **参与方身份三段式硬绑定**：`controller_did` ↔ `participant_id` ↔ `operator`；
   - **委托锚定语义**（S16）：`issuer_did` = 信任根；`delegated_by_did` = 代理方；
   - **VC 存储不在本 crate**：由 `did-sdk<Server>` 管理；
   - **无治理接口**（S18）：`register_participant` 等属 `chain-owner-sdk`；
   - **按角色装配**（S17）：`capabilities.did_resolution` / `vc_wallet` /
     `issuer_signing` 配置项；
   - **环境变量表**（§5.3）；
   - **平台支持矩阵**；musl 静态链接要求；
   - **模式切换**（`CHAIN_SDK_MODE`），切换需重启；
   - **`UserRejected` 透传**；
   - **明确不提供 Python / Node / WASM 绑定**；
   - **未来扩展**：`keyvault-device` / `keyvault-server` 通过 `scheme` 切换。
8. 在 `docs/acceptance-checklist.md` 中按 §8 逐项打勾（S1–S20 + S1-static +
   S2-UserRejected + S3-AppAlias），并记录验证命令输出摘要。K1–K9 引用
   `did-sdk` 仓库结果。

---

## 11. 必须提前对齐的接口决策（agent 不得偏离）

1. **两角色，线性包含**：`SdkRole` 只有 `Verifier` 与 `Issuer`；`Issuer ⊃ Verifier`。
   `role = "app"` 通过 serde `alias` 映射为 `Issuer`（S3-AppAlias）。
   **不得**引入 `VerifierIssuer` / `App` 作为独立变体。
2. **`ChainRole` 与 `SdkRole` 是两个概念**：`ChainRole` 是链上注册表字段
   （`ParticipantRecord.chain_role`），`SdkRole` 是 SDK 构造配置。不得合并。
3. **`KeyVault` 加载策略**：`SdkRole::Issuer` 加载；`SdkRole::Verifier` 不加载。
   `resolve_config` 只在 `role == Issuer` 时解析密码。
4. **`did-sdk` 装配策略（S17）**：两角色都装配 `did-resolution`；
   `issuer-signing` 仅 `Issuer`；`vc-wallet` 按 `capabilities.vc_wallet`。
   **不得**把 `did-sdk` 的 Client / Server 模式选择暴露给 `chain-sdk` 配置——
   由 `did_adapter.rs` 内部决定。
5. **两类密钥 + namespace 隔离（S14）**：
   - `chain-account/` 前缀用于 sr25519，仅 `sign_extrinsic` 函数使用；
   - `participant-did/` 前缀用于 ed25519，仅 `sign_participant_assertion_inner` 使用；
   - 两个函数是**唯一入口**；前缀不匹配返回 `KeyNamespaceViolation`；
   - 类型级不可混用（`Sr25519Signature` vs `Ed25519Signature`）。
6. **写方法入口检查顺序**：
   - `create_anchor` / `revoke_anchor` / `renew_anchor` → 先 `ensure_issuer()?`
   - `update_authorized_domains` / `update_controller_did` → 先 `ensure_self(pid)?`
     （`ensure_self` 内部先调 `ensure_issuer`，再校验 `pid == self.participant_id`，
     不等返回 `Unauthorized`，S19）
   - `sign_participant_assertion` → 先 `ensure_signer()?`（Verifier 返回
     `SignerUnavailable`，S9 / S17）
   - 上述检查**必须先于**后端逻辑与 `vault.sign` 调用
7. **区分 `Unauthorized` / `SignerUnavailable` / `Unsupported`**：
   - `Verifier` 调写方法 → `Unauthorized`
   - `Verifier` 调 `sign_participant_assertion` → `SignerUnavailable`
   - `Issuer` 调 `update_*` 修改他人 → `Unauthorized`（S19）
   - `Issuer` 调写方法但 `Chain` 后端未实现 → `Unsupported`
   - 测试必须分别断言，不得混用。
8. **委托锚定（S16）**：`AnchorRecord.issuer_did` 是语义签发者（**信任根**）；
   `submitter_id` 是提交者（治理审计）；`delegated_by_did` 是代理方（透明审计）。
   `chain-sdk` 的 `create_anchor` 由 Issuer 自身提交，`issuer_did = controller_did`、
   `delegated_by_did = None`；**委托锚定由 `chain-owner-sdk.submit_delegated_anchor`
   提交**，本 crate 只读。
9. **`business_types` 不参与鉴权（S20）**：权限判定只依赖 `chain_role` +
   `authorized_domains`；`role_gate.rs` **不得**读取 `business_types`。
10. **`ChainSdkError::UserRejected` 必须存在**，通过 `map_vault_err` 从
    `KeyVaultError::UserRejected` 1:1 透传。
11. **`valid_until == 0` 表示永久**；`renew_anchor` 对永久锚定返回
    `AnchorPermanent`；`revoke_anchor` 对已撤销返回 `AnchorRevoked`。三后端一致。
12. **`subscribe_event` 是同步方法**：用 `std::sync::RwLock` 维护回调表；
    `emit()` 在 async 上下文里短时间持有锁、不跨 `.await`。
13. **`Centralized` 写签名语义**：`op || \n || canonical_json(body)` 作为待签名
    字节；签名以 `x-chain-signature` 头附加；签名入口必须是
    `keys::sign_extrinsic`（强制 `chain-account/` namespace）。
14. **`Chain` 后端 POC 范围**：接口、错误码、角色拦截、事件订阅订阅必须完整；
    subxt 查询与 extrinsic 提交允许 `Unsupported` 占位；**但写方法必须先
    `ensure_issuer` / `ensure_self` / `ensure_signer`**，保证 S9 / S17 / S19 在
    `chain` 模式下也通过。
15. **musl 静态链接**：`reqwest` 必须 `rustls-tls`；`tokio-tungstenite` 必须
    `rustls-tls-webpki-roots`；禁止 `native-tls` / `openssl` / 未 bundled 的 `-sys`。
16. **`keyvault-core` / `did-sdk` 不在本仓库实现**：不得出现 `keyvault-core/` /
    `did-sdk/` 子目录；不得复制 `KeyVault` trait、`FileKeystore`、`DidSdk` trait
    等定义。所有能力通过外部依赖引入。
17. **VC 存储不在本 crate（S17）**：`ChainSdk` trait **不得**包含
    `store_vc` / `get_vc` / `list_vcs` / `remove_vc` 等方法。这些是 `did-sdk` 的
    职责。
18. **无治理接口（S18）**：`ChainSdk` trait **不得**包含 `register_participant` /
    `deactivate_participant` / `update_operator` / `register_anchor_type` /
    `submit_delegated_anchor`。CI 中 grep 断言。
19. **不提供 Python / Node / WASM 绑定**。若未来需要，应作为**独立仓库**。
20. **未来扩展**：`keyvault-device` / `keyvault-server` 通过 `scheme` 切换，
    只新增 crate + 在 `build_vault` 加 feature 门控分支；`ChainSdk` trait、
    `SdkRole` 不改动。

---

## 12. 与 v1 文档的差异摘要（供评审参考）

| 维度 | v1（旧版） | v2（本版） |
|---|---|---|
| 角色 | `Verifier` / `App` / `Issuer`（三值） | `Verifier` / `Issuer`（两值，线性包含；`app` 为 `issuer` alias） |
| 参与方数据模型 | `IssuerRecord`（issuer_id / issuer_type） | `ParticipantRecord`（participant_id / controller_did / operator / chain_role / business_types） |
| 锚定数据模型 | `issuer_id`（提交者兼签发者） | `submitter_id` + `issuer_did` + `delegated_by_did` 三字段分离 |
| 参与方读方法 | `get_issuer` / `list_issuers` | `get_participant` / `get_participant_by_did` / `list_participants` |
| DID 方法 | `create_offchain_did` / `resolve_did` | 移除 `create_offchain_did`；新增 `resolve_did_key` / `resolve_did_web` |
| 管理写方法 | `update_authorized_domains(issuer_id)` / `update_controller(issuer_id)` | `update_authorized_domains(participant_id)` / `update_controller_did(participant_id)`，**仅自身** |
| 断言签名 | 无 | 新增 `sign_participant_assertion`（仅 Issuer） |
| 密钥管理 | 单一链账户私钥 | **两类服务端密钥 + 两个 namespace**（S14） |
| 签名方法 | `vault.sign` 直接用 | `keys::sign_extrinsic` / `keys::sign_participant_assertion_inner` 类型级隔离 |
| did-sdk 装配 | 不依赖 | **按角色装配**：`did-resolution` / `vc-wallet` / `issuer-signing`（S17） |
| VC 存储 | 提及 | **明确不在本 crate**（S17） |
| 委托锚定 | 无 | 新增：`issuer_did` = 信任根；`delegated_by_did` = 代理方（S16） |
| 身份绑定 | 弱 | **三段式硬绑定**：`controller_did` ↔ `participant_id` ↔ `operator`（S15） |
| 错误码 | `IssuerNotFound` / `IssuerInactive` | `ParticipantNotFound` / `ParticipantDidNotFound` / `ParticipantInactive` / `KeyNamespaceViolation` / `SignerUnavailable` |
| 验收清单 | S1–S13 | **S1–S20**（新增 S14–S20） |
| 治理接口约束 | 提及 | **S18 强制**：trait 不得含治理方法，CI grep 断言 |
| business_types | 无 | 新增枚举，**不参与鉴权**（S20） |