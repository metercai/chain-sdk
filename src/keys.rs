//! 两类服务端密钥的 namespace 管理与**唯一签名入口**（S14）。
//!
//! | namespace | 算法 | 入口 | 返回类型 |
//! |---|---|---|---|
//! | `chain-account/` | sr25519 | [`sign_extrinsic`] | [`Sr25519Signature`] |
//! | `participant-did/` | ed25519 | [`sign_participant_assertion_inner`] | [`Ed25519Signature`] |
//!
//! 两个函数在**调用 `KeyVault::sign_checked` 之前**先校验 `KeyId` 前缀：
//! 不匹配返回 [`ChainSdkError::KeyNamespaceViolation`]，不会触达密钥库。
//! 同时 `keyvault-core` 自身也校验 namespace + algorithm，形成两层防护。
//!
//! # 供 `chain-owner-sdk` 复用
//!
//! [`sign_extrinsic`] 是公开的底层原语；调用方必须自行完成治理层鉴权。
//! `chain-sdk` 的三个后端在调用前都会先执行角色门禁。

use keyvault_core::{KeyAlgorithm, KeyId, KeyNamespace, KeyVault};

use crate::config::KeyVaultKeys;
use crate::error::ChainSdkError;
use crate::types::{AssertionPurpose, Ed25519Signature, Sr25519Signature};

/// `chain-account/` 前缀（sr25519）。
pub const CHAIN_ACCOUNT_PREFIX: &str = "chain-account/";

/// `participant-did/` 前缀（ed25519）。
pub const PARTICIPANT_DID_PREFIX: &str = "participant-did/";

/// 已校验前缀的两类密钥 `KeyId`（仅 Issuer 持有）。
#[derive(Debug, Clone)]
pub(crate) struct LoadedKeys {
    /// `chain-account/` 密钥（P5/P6 的 Centralized 写请求签名使用）。
    #[allow(dead_code)]
    pub(crate) chain_account: KeyId,
    /// `participant-did/` 密钥。
    pub(crate) participant_did: KeyId,
}

/// 校验配置中的两类密钥前缀（S14），返回 [`LoadedKeys`]。
///
/// **不做** `vault.load`：`load` 是异步的，由 [`crate::ChainSdk::health_check`]
/// 或第一次签名时触发；缺失密钥经 [`crate::error::map_vault_err`] 映射为
/// `SignerUnavailable`。
pub(crate) fn load_keys(
    _vault: &dyn KeyVault,
    keys: &KeyVaultKeys,
) -> Result<LoadedKeys, ChainSdkError> {
    validate_configured_keys(keys)?;
    Ok(LoadedKeys {
        chain_account: KeyId(keys.operator.clone()),
        participant_did: KeyId(keys.participant_did.clone()),
    })
}

/// 仅校验配置中的两个 `KeyId` 前缀，不构造 [`LoadedKeys`]。
pub(crate) fn validate_configured_keys(keys: &KeyVaultKeys) -> Result<(), ChainSdkError> {
    validate_key_id(&keys.operator, KeyNamespace::ChainAccount)?;
    validate_key_id(&keys.participant_did, KeyNamespace::ParticipantDid)?;
    Ok(())
}

fn validate_key_id(key: &str, expected: KeyNamespace) -> Result<KeyId, ChainSdkError> {
    let key_id = KeyId(key.to_string());
    if key_id.namespace() != Some(expected) {
        return Err(ChainSdkError::KeyNamespaceViolation(format!(
            "key {key} must start with {} (expected {expected:?})",
            expected.prefix()
        )));
    }
    Ok(key_id)
}

/// **唯一入口**：sr25519 签名（extrinsic / HTTP 写请求体）。
///
/// 强制 namespace 校验：拒绝 `participant-did/` 与任何未知前缀。
pub async fn sign_extrinsic(
    vault: &dyn KeyVault,
    key: &KeyId,
    call: &[u8],
) -> Result<Sr25519Signature, ChainSdkError> {
    if key.namespace() != Some(KeyNamespace::ChainAccount) {
        return Err(ChainSdkError::KeyNamespaceViolation(format!(
            "sign_extrinsic requires {CHAIN_ACCOUNT_PREFIX} namespace; got {}",
            key.0
        )));
    }
    let signature = vault
        .sign_checked(key, call, KeyNamespace::ChainAccount, KeyAlgorithm::Sr25519)
        .await
        .map_err(crate::error::map_vault_err)?;
    Ok(Sr25519Signature(signature.0))
}

/// **唯一入口**：ed25519 参与方断言签名。
///
/// 强制 namespace 校验：拒绝 `chain-account/` 与任何未知前缀。
pub async fn sign_participant_assertion_inner(
    vault: &dyn KeyVault,
    key: &KeyId,
    payload: &[u8],
    _purpose: AssertionPurpose,
) -> Result<Ed25519Signature, ChainSdkError> {
    if key.namespace() != Some(KeyNamespace::ParticipantDid) {
        return Err(ChainSdkError::KeyNamespaceViolation(format!(
            "sign_participant_assertion requires {PARTICIPANT_DID_PREFIX} namespace; got {}",
            key.0
        )));
    }
    let signature = vault
        .sign_checked(
            key,
            payload,
            KeyNamespace::ParticipantDid,
            KeyAlgorithm::Ed25519,
        )
        .await
        .map_err(crate::error::map_vault_err)?;
    Ok(Ed25519Signature(signature.0))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use keyvault_core::{KeyId, KeyVault};

    use super::{load_keys, sign_extrinsic, sign_participant_assertion_inner};
    use crate::ChainSdkError;
    use crate::config::KeyVaultKeys;
    use crate::types::AssertionPurpose;

    #[test]
    fn load_keys_rejects_unknown_prefix() {
        let vault = keyvault_core::MemoryKeystore::new();
        let keys = KeyVaultKeys {
            operator: "not-a-namespace/key".into(),
            participant_did: "participant-did/ok".into(),
        };
        assert!(matches!(
            load_keys(&vault, &keys),
            Err(ChainSdkError::KeyNamespaceViolation(_))
        ));
    }

    #[tokio::test]
    async fn sign_extrinsic_rejects_participant_did_namespace() {
        let vault = keyvault_core::MemoryKeystore::new();
        let bad = KeyId(format!("{}foo", super::PARTICIPANT_DID_PREFIX));
        assert!(matches!(
            sign_extrinsic(&vault, &bad, b"payload").await,
            Err(ChainSdkError::KeyNamespaceViolation(_))
        ));
    }

    #[tokio::test]
    async fn sign_participant_assertion_rejects_chain_account_namespace() {
        let vault = keyvault_core::MemoryKeystore::new();
        let bad = KeyId(format!("{}operator-1", super::CHAIN_ACCOUNT_PREFIX));
        assert!(matches!(
            sign_participant_assertion_inner(
                &vault,
                &bad,
                b"payload",
                AssertionPurpose::VcIssuance
            )
            .await,
            Err(ChainSdkError::KeyNamespaceViolation(_))
        ));
    }

    #[tokio::test]
    async fn signing_succeeds_with_generated_keys() {
        use keyvault_core::{KeyAlgorithm, KeyNamespace};

        let vault = keyvault_core::MemoryKeystore::new();
        let chain = vault
            .generate(KeyNamespace::ChainAccount, KeyAlgorithm::Sr25519)
            .await
            .unwrap();
        let did = vault
            .generate(KeyNamespace::ParticipantDid, KeyAlgorithm::Ed25519)
            .await
            .unwrap();

        assert_eq!(
            sign_extrinsic(&vault, &chain, b"call")
                .await
                .unwrap()
                .0
                .len(),
            64
        );
        assert_eq!(
            sign_participant_assertion_inner(&vault, &did, b"assert", AssertionPurpose::VcIssuance)
                .await
                .unwrap()
                .0
                .len(),
            64
        );
    }
}
