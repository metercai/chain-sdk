//! 统一错误类型与 `keyvault-core` 错误映射。
//!
//! **映射必须唯一**：所有后端只通过 [`map_vault_err`] 把
//! `keyvault_core::KeyVaultError` 转成 [`ChainSdkError`]，不得各自 `match`。

use crate::types::AnchorType;

/// `chain-sdk` 对外错误。
///
/// 与 `did-sdk::DidSdkError` 保持风格一致：错误变体承载足够的诊断信息，
/// 但**不包含**私钥、密码或密文。
#[derive(Debug, thiserror::Error)]
pub enum ChainSdkError {
    /// 锚定不存在。
    #[error("Anchor not found: {0}")]
    AnchorNotFound(String),
    /// 锚定类型未在链上注册。
    #[error("Anchor type not registered: {0:?}")]
    AnchorTypeNotRegistered(AnchorType),
    /// 永久锚定（`valid_until == 0`）不能续期。
    #[error("Anchor is permanent: {0}")]
    AnchorPermanent(String),
    /// 锚定已撤销，不能重复撤销。
    #[error("Anchor already revoked: {0}")]
    AnchorRevoked(String),
    /// 参与方不存在。
    #[error("Participant not found: {0}")]
    ParticipantNotFound(u32),
    /// 参与方 `controller_did` 未注册。
    #[error("Participant DID not found: {0}")]
    ParticipantDidNotFound(String),
    /// 参与方已停用。
    #[error("Participant inactive: {0}")]
    ParticipantInactive(u32),
    /// 角色或自身范围越权（S9 / S19）。
    #[error("Unauthorized: {0}")]
    Unauthorized(String),
    /// 密钥 namespace 前缀不匹配（S14）。
    #[error("Key namespace violation: {0}")]
    KeyNamespaceViolation(String),
    /// 当前角色没有对应签名器（Verifier 调断言签名）。
    #[error("Signer not available in role: {0}")]
    SignerUnavailable(String),
    /// 链后端不可用。
    #[error("Chain unavailable: {0}")]
    ChainUnavailable(String),
    /// 网络错误（HTTP / WebSocket / RPC）。
    #[error("Network error: {0}")]
    NetworkError(String),
    /// 参数非法。
    #[error("Invalid argument: {0}")]
    InvalidArgument(String),
    /// 签名失败。
    #[error("Signing failed: {0}")]
    SigningFailed(String),
    /// 用户拒绝签名。来源：`KeyVaultError::UserRejected` 1:1 透传。
    #[error("User rejected signing")]
    UserRejected,
    /// 配置错误。
    #[error("Configuration error: {0}")]
    ConfigError(String),
    /// 当前后端/阶段不支持该操作。
    #[error("Operation not supported: {0}")]
    Unsupported(String),
}

/// 把 [`keyvault_core::KeyVaultError`] 映射为 [`ChainSdkError`]。
///
/// 注意：`keyvault-core` 0.1.0 的 `KeyVaultError` 比规格书多两个变体
/// （`NamespaceViolation` / `AlgorithmMismatch`），此处完整覆盖，避免
/// 非穷尽匹配编译失败。
#[allow(dead_code)] // P0 起由 keys.rs 与三个后端统一使用。
pub(crate) fn map_vault_err(error: keyvault_core::KeyVaultError) -> ChainSdkError {
    use keyvault_core::KeyVaultError as KE;
    match error {
        KE::KeyNotFound(message) => {
            ChainSdkError::SignerUnavailable(format!("key not found: {message}"))
        }
        KE::KeyAlreadyExists(message) => {
            ChainSdkError::InvalidArgument(format!("key exists: {message}"))
        }
        KE::UserRejected => ChainSdkError::UserRejected,
        KE::InvalidPassword => ChainSdkError::SignerUnavailable("invalid keystore password".into()),
        KE::Locked => ChainSdkError::SignerUnavailable("keystore locked".into()),
        KE::SigningFailed(message) => ChainSdkError::SigningFailed(message),
        KE::IoError(message) => ChainSdkError::ChainUnavailable(message),
        KE::InvalidKeyFormat(message) => ChainSdkError::InvalidArgument(message),
        KE::DecryptionFailed => ChainSdkError::SigningFailed("decryption failed".into()),
        KE::SerializationError(message) => ChainSdkError::InvalidArgument(message),
        KE::Unsupported => ChainSdkError::InvalidArgument("unsupported keystore operation".into()),
        KE::NamespaceViolation {
            key,
            actual,
            expected,
        } => ChainSdkError::KeyNamespaceViolation(format!(
            "key {key} is {actual:?}, expected {expected:?}"
        )),
        KE::AlgorithmMismatch {
            key,
            actual,
            expected,
        } => ChainSdkError::InvalidArgument(format!(
            "key {key} algorithm is {actual:?}, expected {expected:?}"
        )),
    }
}
