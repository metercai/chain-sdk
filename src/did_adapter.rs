//! 按角色装配 `did-sdk<Server>`，并把它收敛为内部 [`DidPort`]。
//!
//! 为什么不直接持有 `Arc<dyn did_sdk::DidSdk>`：`did-sdk 0.1.0` 的
//! `DidSdk<M: SdkMode>` 是**具体泛型结构**，不是对象安全 trait；能力也按
//! `Client` / `Server` 分别 impl（`resolve_did_web` 只在 `Server`）。
//!
//! 约定（D-02 / D-05）：
//!
//! - `did-sdk` 以 `key_vault = None` 构造：参与方签名私钥只在 `chain-sdk` 侧；
//! - `did-resolution` 两角色都装配；`vc-wallet` 由 `did-sdk` 自身持有存储；
//! - `resolve_did`（链上记录）由 `chain-sdk` 的三后端实现，**不**委托 did-sdk；
//! - `resolve_did_key` 用 `keyvault-core` 离线解码，不依赖 did-sdk 的 Client 模式。

use std::sync::Arc;

use async_trait::async_trait;
use did_sdk::{DidSdk, DidSdkMode, Server, ServerConfig, build_server_sdk};

use crate::config::{ChainSdkConfig, ChainSdkMode};
use crate::{ChainSdkError, DidDocument, Ed25519PublicKey, VerificationMethod};

/// `chain-sdk` 需要的 DID 解析能力（可注入 mock 以便测试）。
#[async_trait]
pub(crate) trait DidPort: Send + Sync {
    /// 离线解析 `did:key` 的 Ed25519 公钥。
    fn resolve_did_key(&self, did: &str) -> Result<Ed25519PublicKey, ChainSdkError>;

    /// 通过 HTTPS 解析 `did:web` 文档。
    async fn resolve_did_web(&self, did: &str) -> Result<DidDocument, ChainSdkError>;

    /// 校验选择性披露 VP（holder 签名、challenge、VC 签名、Disclosure 哈希）。
    ///
    /// 锚定查询不在此处执行（由 `chain-sdk` 负责，R1）。
    async fn verify_vp(
        &self,
        presentation: &did_sdk::VerifiablePresentation,
        expected_challenge: &str,
    ) -> Result<did_sdk::VpVerificationResult, ChainSdkError>;
}

/// 把 `DidSdkError` 映射为 `ChainSdkError`（selective disclosure 错误保留语义）。
pub(crate) fn map_did_err(error: did_sdk::DidSdkError) -> ChainSdkError {
    use did_sdk::DidSdkError as DE;
    match error {
        DE::DisclosureHashMismatch(name) => {
            let _ = name;
            ChainSdkError::DisclosureHashMismatch
        }
        DE::DisclosureDuplicate(name) => ChainSdkError::DisclosureDuplicate(name),
        DE::DisclosureMissing(message) => ChainSdkError::DisclosureMissing(message),
        DE::SdAlgUnsupported(alg) => ChainSdkError::SdAlgUnsupported(alg),
        other => ChainSdkError::InvalidArgument(format!("did-sdk: {other}")),
    }
}

/// 基于 `did-sdk<Server>` 的适配器。
pub(crate) struct DidSdkAdapter {
    sdk: Arc<DidSdk<Server>>,
}

impl DidSdkAdapter {
    /// 按 `chain-sdk` 配置装配 did-sdk。
    ///
    /// 要求 `participant.controller_did` 非空（由 `validate_for_build` 保证）。
    pub(crate) fn build(config: &ChainSdkConfig) -> Result<Self, ChainSdkError> {
        let mode = match config.mode {
            ChainSdkMode::Mock => DidSdkMode::Mock,
            ChainSdkMode::Centralized | ChainSdkMode::Chain => DidSdkMode::Centralized,
        };
        let server = ServerConfig {
            mode,
            participant_did: config.participant.controller_did.clone(),
            vc_storage_path: config.did.vc_storage_path.clone(),
            key_vault: None,
            resolved_password: None,
            centralized_endpoint: config.centralized_endpoint.clone(),
            cache_ttl_secs: config.cache_ttl_secs,
        };
        let sdk = build_server_sdk(&server)
            .map_err(|error| ChainSdkError::ConfigError(format!("did-sdk: {error}")))?;
        Ok(Self { sdk })
    }

    /// 底层 did-sdk 句柄（P11 的 VC 组装/校验可能用到）。
    #[allow(dead_code)]
    pub(crate) fn sdk(&self) -> &Arc<DidSdk<Server>> {
        &self.sdk
    }
}

#[async_trait]
impl DidPort for DidSdkAdapter {
    fn resolve_did_key(&self, did: &str) -> Result<Ed25519PublicKey, ChainSdkError> {
        keyvault_core::crypto::decode_did_key_pubkey(did)
            .map(Ed25519PublicKey)
            .map_err(|error| ChainSdkError::InvalidArgument(format!("did:key: {error}")))
    }

    async fn resolve_did_web(&self, did: &str) -> Result<DidDocument, ChainSdkError> {
        let document =
            self.sdk.resolve_did_web(did).await.map_err(|error| {
                ChainSdkError::InvalidArgument(format!("did:web {did}: {error}"))
            })?;
        Ok(convert_document(document))
    }

    async fn verify_vp(
        &self,
        presentation: &did_sdk::VerifiablePresentation,
        expected_challenge: &str,
    ) -> Result<did_sdk::VpVerificationResult, ChainSdkError> {
        self.sdk
            .verify_vp(presentation, expected_challenge)
            .await
            .map_err(map_did_err)
    }
}

fn convert_document(document: did_sdk::DidDocument) -> DidDocument {
    DidDocument {
        id: document.id,
        verification_methods: document
            .verification_method
            .into_iter()
            .map(|method| VerificationMethod {
                id: method.id,
                controller: method.controller,
                public_key_bytes: method
                    .public_key_multibase
                    .as_deref()
                    .and_then(|encoded| did_sdk::did::decode_multibase_key(encoded).ok())
                    .map(|key| key.to_vec())
                    .unwrap_or_default(),
            })
            .collect(),
    }
}
