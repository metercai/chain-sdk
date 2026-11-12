//! 初始化阶段的占位后端。
//!
//! 该实现**不是**最终交付：读方法返回 [`ChainSdkError::Unsupported`]，写方法在
//! 执行角色门禁后返回 `Unsupported`。断言签名与 `did:key` 解析已经可用。
//!
//! P1（Mock）、P5（Centralized）、P10（Chain）分别用真实后端替换本模块；
//! 替换时必须保持角色门禁顺序不变。

use async_trait::async_trait;

use crate::role_gate::RoleGate;
use crate::{
    AnchorRecord, AnchorType, AssertionPurpose, ChainSdk, ChainSdkError, DidDocument, DidRecord,
    Ed25519PublicKey, Ed25519Signature, EventData, HealthStatus, ParticipantRecord, SdkCore,
    SdkRole, Unsubscribe,
};

/// 占位后端：仅用于让工厂与配置路径可运行。
pub(crate) struct PlaceholderChainSdk {
    core: SdkCore,
}

impl PlaceholderChainSdk {
    pub(crate) fn new(core: SdkCore) -> Self {
        Self { core }
    }
}

impl RoleGate for PlaceholderChainSdk {
    fn role(&self) -> SdkRole {
        self.core.role
    }

    fn self_participant_id(&self) -> u32 {
        self.core.config.participant_id
    }
}

fn pending(what: &str) -> ChainSdkError {
    ChainSdkError::Unsupported(format!(
        "{what} 尚未实现：按 docs/implementation-plan.md 的 P1/P5/P10 落地"
    ))
}

#[async_trait]
impl ChainSdk for PlaceholderChainSdk {
    async fn health_check(&self) -> Result<HealthStatus, ChainSdkError> {
        Ok(HealthStatus {
            healthy: false,
            mode: self.core.config.mode.as_str().to_string(),
            detail: Some("backend implementation pending (P1/P5/P10)".into()),
            block_number: None,
        })
    }

    async fn get_anchor(&self, _anchor_id: &str) -> Result<AnchorRecord, ChainSdkError> {
        Err(pending("get_anchor"))
    }

    async fn list_anchors(
        &self,
        _anchor_type: Option<AnchorType>,
        _submitter_id: Option<u32>,
    ) -> Result<Vec<AnchorRecord>, ChainSdkError> {
        Err(pending("list_anchors"))
    }

    async fn get_participant(
        &self,
        _participant_id: u32,
    ) -> Result<ParticipantRecord, ChainSdkError> {
        Err(pending("get_participant"))
    }

    async fn get_participant_by_did(
        &self,
        _did: &str,
    ) -> Result<Option<ParticipantRecord>, ChainSdkError> {
        Err(pending("get_participant_by_did"))
    }

    async fn list_participants(&self) -> Result<Vec<ParticipantRecord>, ChainSdkError> {
        Err(pending("list_participants"))
    }

    async fn resolve_did(&self, _did: &str) -> Result<Option<DidRecord>, ChainSdkError> {
        Err(pending("resolve_did"))
    }

    fn resolve_did_key(&self, did: &str) -> Result<Ed25519PublicKey, ChainSdkError> {
        keyvault_core::crypto::decode_did_key_pubkey(did)
            .map(Ed25519PublicKey)
            .map_err(|error| ChainSdkError::InvalidArgument(format!("did:key: {error}")))
    }

    async fn resolve_did_web(&self, _did: &str) -> Result<DidDocument, ChainSdkError> {
        Err(pending("resolve_did_web"))
    }

    async fn create_anchor(
        &self,
        _anchor_type: AnchorType,
        _payload_hash: &str,
        _valid_until: u64,
    ) -> Result<String, ChainSdkError> {
        self.ensure_issuer()?;
        Err(pending("create_anchor"))
    }

    async fn revoke_anchor(&self, _anchor_id: &str) -> Result<(), ChainSdkError> {
        self.ensure_issuer()?;
        Err(pending("revoke_anchor"))
    }

    async fn renew_anchor(
        &self,
        _anchor_id: &str,
        _new_valid_until: u64,
    ) -> Result<(), ChainSdkError> {
        self.ensure_issuer()?;
        Err(pending("renew_anchor"))
    }

    async fn update_authorized_domains(
        &self,
        participant_id: u32,
        _domains: Vec<String>,
    ) -> Result<(), ChainSdkError> {
        self.ensure_self(participant_id)?;
        Err(pending("update_authorized_domains"))
    }

    async fn update_controller_did(
        &self,
        participant_id: u32,
        _new_controller_did: &str,
    ) -> Result<(), ChainSdkError> {
        self.ensure_self(participant_id)?;
        Err(pending("update_controller_did"))
    }

    async fn sign_participant_assertion(
        &self,
        payload: &[u8],
        purpose: AssertionPurpose,
    ) -> Result<Ed25519Signature, ChainSdkError> {
        self.ensure_signer()?;
        let vault =
            self.core.vault.as_ref().ok_or_else(|| {
                ChainSdkError::SignerUnavailable("issuer requires KeyVault".into())
            })?;
        let keys = self
            .core
            .keys
            .as_ref()
            .ok_or_else(|| ChainSdkError::SignerUnavailable("keys not loaded".into()))?;
        crate::keys::sign_participant_assertion_inner(
            vault.as_ref(),
            &keys.participant_did,
            payload,
            purpose,
        )
        .await
    }

    fn subscribe_event(
        &self,
        _event_name: &str,
        _callback: Box<dyn Fn(EventData) + Send + Sync>,
    ) -> Result<Unsubscribe, ChainSdkError> {
        Err(pending("subscribe_event"))
    }
}
