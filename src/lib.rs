//! 链能力统一契约（`ChainSdk`）与公开数据模型。
//!
//! # 角色模型
//!
//! `chain-sdk` 只暴露两种**配置角色**（[`SdkRole`]）：
//!
//! - [`SdkRole::Verifier`]：只读（锚定、参与方、DID 查询）；
//! - [`SdkRole::Issuer`]：**线性包含** Verifier 的全部读能力，额外加载两类
//!   服务端密钥并允许写操作与断言签名（`Verifier ⊂ Issuer`）。
//!
//! 旧配置 `role = "app"` 通过 serde `alias` 映射为 `Issuer`，不是独立变体。
//!
//! # 两类服务端密钥与两个 namespace（S14）
//!
//! - `chain-account/`：sr25519，用于 extrinsic / HTTP 写请求体签名（内部
//!   `keys::sign_extrinsic`）；
//! - `participant-did/`：ed25519，用于参与方 `did:web` 断言签名
//!   （[`ChainSdk::sign_participant_assertion`]）。
//!
//! 两个签名入口在类型上不可互换，且入口处强制 namespace 前缀校验；
//! 前缀不匹配返回 [`ChainSdkError::KeyNamespaceViolation`]。
//!
//! # 职责边界
//!
//! - **VC 存储不在本 crate**：由 `did-sdk<Server>` 持有侧管理（S17）；
//! - **治理接口不在本 crate**：`register_participant` / `deactivate_participant` /
//!   `update_operator` / `register_anchor_type` / `submit_delegated_anchor`
//!   属于 `chain-owner-sdk`（S18）；
//! - **锚定查询在本 crate**：`did-sdk` 不提供 `get_anchor`（R1 修订）。
//!
//! # 实现状态
//!
//! 本提交是**初始化骨架**：公开契约已冻结，后端实现与测试按
//! `docs/implementation-plan.md` 分阶段进行（P0–P12），评审通过后开始实现。

pub mod config;
pub mod error;
pub mod events;
pub mod keys;
pub mod types;

mod placeholder;
mod role_gate;

use std::sync::Arc;

use async_trait::async_trait;

pub use config::{
    Capabilities, ChainSdkConfig, ChainSdkMode, ConfigOverrides, DidConfig, KeyVaultConfig,
    KeyVaultKeys, ParticipantConfig, SdkRole, build_chain_sdk, resolve_config,
};
pub use error::ChainSdkError;
pub use events::{EventData, Unsubscribe};
pub use types::{
    AnchorRecord, AnchorType, AssertionPurpose, BusinessType, ChainRole, DidDocument, DidRecord,
    Ed25519PublicKey, Ed25519Signature, HealthStatus, ParticipantRecord, Sr25519Signature,
    VerificationMethod,
};

/// 三个后端共享的构造状态。
///
/// - `vault`：Verifier 为 `None`；Issuer 加载 `file-keystore` / `memory-keystore`；
/// - `keys`：Issuer 才持有（已校验前缀）；
/// - `config` / `role`：构造时快照，运行期不变。
pub(crate) struct SdkCore {
    /// 密钥库（仅 Issuer）。
    pub(crate) vault: Option<Arc<dyn keyvault_core::KeyVault>>,
    /// 构造配置。
    pub(crate) config: ChainSdkConfig,
    /// 配置角色。
    pub(crate) role: SdkRole,
    /// 两类密钥（仅 Issuer）。
    pub(crate) keys: Option<crate::keys::LoadedKeys>,
}

/// 链能力的统一契约：三种后端（Mock / Centralized / Chain）× 两种角色
/// （Verifier / Issuer）。
///
/// # 越权行为（三后端一致）
///
/// | 调用 | Verifier | Issuer |
/// |---|---|---|
/// | 写方法（`create_anchor` 等） | `Unauthorized` | 允许（Chain 未实现时 `Unsupported`） |
/// | `update_*` 修改他人 | `Unauthorized` | `Unauthorized`（S19） |
/// | `sign_participant_assertion` | `SignerUnavailable` | 允许 |
///
/// # 对象安全
///
/// 本 trait 通过 `Arc<dyn ChainSdk>` 使用；`subscribe_event` 是同步方法，
/// 回调表由实现内部以 `std::sync::RwLock` 维护。
#[async_trait]
pub trait ChainSdk: Send + Sync {
    // ===== 连接管理 =====

    /// 健康检查。返回当前后端模式、是否可用以及（可选）区块高度。
    async fn health_check(&self) -> Result<HealthStatus, ChainSdkError>;

    // ===== 锚定读（两角色均可） =====

    /// 按 `anchor_id` 查询锚定记录。
    async fn get_anchor(&self, anchor_id: &str) -> Result<AnchorRecord, ChainSdkError>;

    /// 列出锚定记录，可按锚定类型与提交者过滤。
    async fn list_anchors(
        &self,
        anchor_type: Option<AnchorType>,
        submitter_id: Option<u32>,
    ) -> Result<Vec<AnchorRecord>, ChainSdkError>;

    // ===== 参与方读（两角色均可） =====

    /// 按链上 `participant_id` 查询参与方记录。
    async fn get_participant(
        &self,
        participant_id: u32,
    ) -> Result<ParticipantRecord, ChainSdkError>;

    /// 按 `controller_did` 查询参与方记录；未注册时返回 `None`。
    async fn get_participant_by_did(
        &self,
        did: &str,
    ) -> Result<Option<ParticipantRecord>, ChainSdkError>;

    /// 列出全部参与方记录。
    async fn list_participants(&self) -> Result<Vec<ParticipantRecord>, ChainSdkError>;

    // ===== DID（两角色均可） =====

    /// 解析链上 DID 记录（`did-sdk` 不提供该能力）。
    async fn resolve_did(&self, did: &str) -> Result<Option<DidRecord>, ChainSdkError>;

    /// 离线解析 `did:key` 的 Ed25519 公钥（自包含，不访问网络）。
    fn resolve_did_key(&self, did: &str) -> Result<Ed25519PublicKey, ChainSdkError>;

    /// 解析 `did:web` 文档（HTTPS `.well-known`）。
    async fn resolve_did_web(&self, did: &str) -> Result<DidDocument, ChainSdkError>;

    // ===== 锚定写（仅 Issuer） =====

    /// 创建锚定，返回新 `anchor_id`。
    ///
    /// `valid_until == 0` 表示永久。
    async fn create_anchor(
        &self,
        anchor_type: AnchorType,
        payload_hash: &str,
        valid_until: u64,
    ) -> Result<String, ChainSdkError>;

    /// 撤销锚定。已撤销返回 [`ChainSdkError::AnchorRevoked`]。
    async fn revoke_anchor(&self, anchor_id: &str) -> Result<(), ChainSdkError>;

    /// 续期锚定。永久锚定返回 [`ChainSdkError::AnchorPermanent`]。
    async fn renew_anchor(
        &self,
        anchor_id: &str,
        new_valid_until: u64,
    ) -> Result<(), ChainSdkError>;

    // ===== 参与方自身管理写（仅 Issuer，且仅自身，S19） =====

    /// 更新自身参与方的授权域名列表；修改他人返回 `Unauthorized`。
    async fn update_authorized_domains(
        &self,
        participant_id: u32,
        domains: Vec<String>,
    ) -> Result<(), ChainSdkError>;

    /// 更新自身参与方的 `controller_did`；修改他人返回 `Unauthorized`。
    async fn update_controller_did(
        &self,
        participant_id: u32,
        new_controller_did: &str,
    ) -> Result<(), ChainSdkError>;

    // ===== 参与方断言签名（仅 Issuer；S9 / S17） =====

    /// 用 `participant-did/`（ed25519）密钥对载荷签名。
    ///
    /// Verifier 调用返回 [`ChainSdkError::SignerUnavailable`]。
    async fn sign_participant_assertion(
        &self,
        payload: &[u8],
        purpose: AssertionPurpose,
    ) -> Result<Ed25519Signature, ChainSdkError>;

    // ===== 事件订阅（两角色均可，S11） =====

    /// 订阅链上事件。返回的句柄被调用时取消订阅（幂等）。
    ///
    /// 本方法是同步方法；实现内部不得在持有回调表锁时跨 `.await`。
    fn subscribe_event(
        &self,
        event_name: &str,
        callback: Box<dyn Fn(EventData) + Send + Sync>,
    ) -> Result<Unsubscribe, ChainSdkError>;
}
