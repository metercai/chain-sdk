//! `Chain` 后端：subxt 连接、健康检查、事件流与角色拦截。
//!
//! # POC 范围（D-06）
//!
//! - **已实现**：连接与健康检查、finalized 区块事件流、角色门禁、`did:key`/`did:web`
//!   解析、参与方断言签名；
//! - **未实现**：storage 查询与 extrinsic 提交返回 [`ChainSdkError::Unsupported`]。
//!   原因：仓库没有 identity-chain 的 pallet / 存储项元数据，且
//!   `subxt::tx::Signer::sign` 没有 `Result`，无法透传 `UserRejected`。
//!   写方法**仍然先执行角色门禁**，因此 S9 / S17 / S19 在 chain 模式成立。
//!
//! 真实链集成测试标注 `#[ignore]`，需要 `CHAIN_SDK_CHAIN_ENDPOINT`。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use subxt::{OnlineClient, SubstrateConfig};
use tokio::sync::OnceCell;

use crate::events::SubscriptionTable;
use crate::role_gate::RoleGate;
use crate::{
    AnchorRecord, AnchorType, AssertionPurpose, ChainSdk, ChainSdkError, DidDocument, DidRecord,
    Ed25519PublicKey, Ed25519Signature, EventData, HealthStatus, ParticipantRecord, SdkCore,
    SdkRole, Unsubscribe,
};

/// 真实 Substrate 链后端（subxt）。
pub struct ChainModeSdk {
    core: SdkCore,
    endpoint: String,
    client: OnceCell<OnlineClient<SubstrateConfig>>,
    subscribers: Arc<SubscriptionTable>,
    events_started: AtomicBool,
}

impl ChainModeSdk {
    pub(crate) fn new(core: SdkCore) -> Self {
        Self {
            endpoint: core.config.chain_endpoint.clone(),
            core,
            client: OnceCell::new(),
            subscribers: Arc::new(SubscriptionTable::new()),
            events_started: AtomicBool::new(false),
        }
    }

    async fn client(&self) -> Result<&OnlineClient<SubstrateConfig>, ChainSdkError> {
        self.client
            .get_or_try_init(|| async {
                OnlineClient::<SubstrateConfig>::from_url(&self.endpoint)
                    .await
                    .map_err(|error| {
                        ChainSdkError::ChainUnavailable(format!(
                            "connect {}: {error}",
                            self.endpoint
                        ))
                    })
            })
            .await
    }

    fn storage_pending(what: &str) -> ChainSdkError {
        ChainSdkError::Unsupported(format!(
            "chain {what} 尚未实现：缺少 identity-chain 元数据（D-06）"
        ))
    }
}

impl ChainModeSdk {
    pub(crate) fn core(&self) -> &SdkCore {
        &self.core
    }
}

impl RoleGate for ChainModeSdk {
    fn role(&self) -> SdkRole {
        self.core.role
    }

    fn self_participant_id(&self) -> u32 {
        self.core.config.participant_id
    }
}

fn start_event_stream(endpoint: String, table: Arc<SubscriptionTable>) {
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn(run_event_stream(endpoint, table));
        return;
    }
    std::thread::spawn(move || {
        if let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            runtime.block_on(run_event_stream(endpoint, table));
        }
    });
}

async fn run_event_stream(endpoint: String, table: Arc<SubscriptionTable>) {
    loop {
        if let Ok(client) = OnlineClient::<SubstrateConfig>::from_url(&endpoint).await {
            if let Ok(mut blocks) = client.stream_blocks().await {
                while let Some(block) = blocks.next().await {
                    let Ok(block) = block else { break };
                    let number = block.number();
                    let Ok(at) = block.at().await else { continue };
                    let Ok(events) = at.events().fetch().await else {
                        continue;
                    };
                    for event in events.iter().flatten() {
                        let event = EventData {
                            event_name: event.event_name().to_string(),
                            anchor_id: None,
                            participant_id: None,
                            block_number: Some(number),
                            payload: serde_json::json!({ "pallet": event.pallet_name() }),
                        };
                        table.emit(&event);
                    }
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

#[async_trait]
impl ChainSdk for ChainModeSdk {
    fn participant_did(&self) -> &str {
        &self.core.config.participant.controller_did
    }

    async fn health_check(&self) -> Result<HealthStatus, ChainSdkError> {
        match self.client().await {
            Ok(client) => match client.at_current_block().await {
                Ok(at) => Ok(HealthStatus {
                    healthy: true,
                    mode: "chain".into(),
                    detail: self
                        .core
                        .config
                        .require_finalized_node
                        .then(|| "using finalized node".to_string()),
                    block_number: Some(at.block_number()),
                }),
                Err(error) => Ok(HealthStatus {
                    healthy: false,
                    mode: "chain".into(),
                    detail: Some(format!("current block: {error}")),
                    block_number: None,
                }),
            },
            Err(error) => Ok(HealthStatus {
                healthy: false,
                mode: "chain".into(),
                detail: Some(error.to_string()),
                block_number: None,
            }),
        }
    }

    async fn get_anchor(&self, _anchor_id: &str) -> Result<AnchorRecord, ChainSdkError> {
        Err(Self::storage_pending("get_anchor"))
    }

    async fn list_anchors(
        &self,
        _anchor_type: Option<AnchorType>,
        _submitter_id: Option<u32>,
    ) -> Result<Vec<AnchorRecord>, ChainSdkError> {
        Err(Self::storage_pending("list_anchors"))
    }

    async fn get_participant(
        &self,
        _participant_id: u32,
    ) -> Result<ParticipantRecord, ChainSdkError> {
        Err(Self::storage_pending("get_participant"))
    }

    async fn get_participant_by_did(
        &self,
        _did: &str,
    ) -> Result<Option<ParticipantRecord>, ChainSdkError> {
        Err(Self::storage_pending("get_participant_by_did"))
    }

    async fn list_participants(&self) -> Result<Vec<ParticipantRecord>, ChainSdkError> {
        Err(Self::storage_pending("list_participants"))
    }

    async fn resolve_did(&self, _did: &str) -> Result<Option<DidRecord>, ChainSdkError> {
        Err(Self::storage_pending("resolve_did"))
    }

    fn resolve_did_key(&self, did: &str) -> Result<Ed25519PublicKey, ChainSdkError> {
        let port = self
            .core
            .did
            .as_ref()
            .ok_or_else(|| ChainSdkError::SignerUnavailable("did-sdk not assembled".into()))?;
        port.resolve_did_key(did)
    }

    async fn resolve_did_web(&self, did: &str) -> Result<DidDocument, ChainSdkError> {
        let port = self
            .core
            .did
            .as_ref()
            .ok_or_else(|| ChainSdkError::SignerUnavailable("did-sdk not assembled".into()))?;
        port.resolve_did_web(did).await
    }

    async fn create_anchor(
        &self,
        _anchor_type: AnchorType,
        _payload_hash: &str,
        _valid_until: u64,
    ) -> Result<String, ChainSdkError> {
        self.ensure_issuer()?;
        Err(Self::storage_pending("create_anchor"))
    }

    async fn revoke_anchor(&self, _anchor_id: &str) -> Result<(), ChainSdkError> {
        self.ensure_issuer()?;
        Err(Self::storage_pending("revoke_anchor"))
    }

    async fn renew_anchor(
        &self,
        _anchor_id: &str,
        _new_valid_until: u64,
    ) -> Result<(), ChainSdkError> {
        self.ensure_issuer()?;
        Err(Self::storage_pending("renew_anchor"))
    }

    async fn update_authorized_domains(
        &self,
        participant_id: u32,
        _domains: Vec<String>,
    ) -> Result<(), ChainSdkError> {
        self.ensure_self(participant_id)?;
        Err(Self::storage_pending("update_authorized_domains"))
    }

    async fn update_controller_did(
        &self,
        participant_id: u32,
        _new_controller_did: &str,
    ) -> Result<(), ChainSdkError> {
        self.ensure_self(participant_id)?;
        Err(Self::storage_pending("update_controller_did"))
    }

    async fn sign_participant_assertion(
        &self,
        payload: &[u8],
        purpose: AssertionPurpose,
    ) -> Result<Ed25519Signature, ChainSdkError> {
        self.ensure_signer()?;
        self.core.sign_participant_assertion(payload, purpose).await
    }

    fn subscribe_event(
        &self,
        event_name: &str,
        callback: Box<dyn Fn(EventData) + Send + Sync>,
    ) -> Result<Unsubscribe, ChainSdkError> {
        let handle = self.subscribers.register(event_name, callback);
        if !self.events_started.swap(true, Ordering::SeqCst) {
            start_event_stream(
                self.core.config.chain_endpoint.clone(),
                Arc::clone(&self.subscribers),
            );
        }
        Ok(handle)
    }
}
