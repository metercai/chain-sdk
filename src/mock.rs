//! `Mock` 后端：进程内状态，无外部依赖。
//!
//! - **读**：锚定、参与方（双向索引）、DID、健康检查；
//! - **持久化**：配置了 `mock.state_path` 时，每次状态变更后原子写 JSON；
//! - **故障注入**（S5）：延迟、不可用、指定操作错误；
//! - **写**：P2 落地（当前先执行角色门禁，再返回 `Unsupported`）；
//! - **事件**：P9 落地。

use std::collections::{HashMap, HashSet};
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_lock::RwLock;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::events::SubscriptionTable;
use crate::role_gate::RoleGate;
use crate::{
    AnchorRecord, AnchorType, AssertionPurpose, ChainSdk, ChainSdkError, DidDocument, DidRecord,
    Ed25519PublicKey, Ed25519Signature, EventData, HealthStatus, ParticipantRecord, SdkCore,
    SdkRole, Unsubscribe,
};

/// Mock 后端的进程内状态。
///
/// JSON 形状即 `tests/fixtures/mock_state.json`（`participants` 的键为字符串形式的
/// `participant_id`）。`current_time` 是 Mock 的内部时间线，`advance_time` 推进。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MockState {
    /// 锚定记录。
    pub anchors: HashMap<String, AnchorRecord>,
    /// 参与方记录。
    pub participants: HashMap<u32, ParticipantRecord>,
    /// `controller_did` → `participant_id` 索引。
    pub did_participant: HashMap<String, u32>,
    /// `operator` → `participant_id` 索引。
    pub account_participant: HashMap<String, u32>,
    /// 已注册的锚定类型。
    pub registered_anchor_types: HashSet<AnchorType>,
    /// 链上 DID 记录。
    pub dids: HashMap<String, DidRecord>,
    /// 内部当前时间（Unix 秒）。
    pub current_time: u64,
    /// 下一个锚定序号。
    pub next_anchor_seq: u64,
}

impl MockState {
    /// 从 JSON 字节解析状态。
    pub fn from_json_slice(raw: &[u8]) -> Result<Self, ChainSdkError> {
        serde_json::from_slice(raw)
            .map_err(|error| ChainSdkError::InvalidArgument(format!("parse mock state: {error}")))
    }

    /// 从 JSON 字符串解析状态。
    pub fn from_json_str(raw: &str) -> Result<Self, ChainSdkError> {
        Self::from_json_slice(raw.as_bytes())
    }

    /// 序列化为紧凑 JSON。
    pub fn to_json_vec(&self) -> Result<Vec<u8>, ChainSdkError> {
        serde_json::to_vec(self).map_err(|error| {
            ChainSdkError::InvalidArgument(format!("serialize mock state: {error}"))
        })
    }
}

/// Mock 故障注入规则（S5）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailureRule {
    /// 指定操作延迟 `millis` 毫秒后成功。
    Latency {
        /// 操作名（如 `get_anchor`）。
        op: String,
        /// 延迟毫秒数。
        millis: u64,
    },
    /// 任意操作都返回 [`ChainSdkError::ChainUnavailable`]。
    Unavailable,
    /// 指定操作返回 [`ChainSdkError::NetworkError`]。
    Specific {
        /// 操作名。
        op: String,
        /// 错误信息。
        error: String,
    },
}

enum FailureOutcome {
    Ok,
    Unavailable,
    Specific(String),
    Latency(u64),
}

/// Shared handle to a [`MockState`].
///
/// Cloning the handle shares the same underlying state; this lets a host (for
/// example `chain-owner-sdk`) perform governance writes that are immediately
/// visible to the wrapped `ChainSdk` instance.
#[derive(Clone)]
pub struct MockStateHandle(Arc<RwLock<MockState>>);

impl MockStateHandle {
    /// Create a new handle from an initial state.
    pub fn new(state: MockState) -> Self {
        Self(Arc::new(RwLock::new(state)))
    }

    /// Clone a snapshot for read-only use.
    pub async fn snapshot(&self) -> MockState {
        self.0.read().await.clone()
    }

    /// Atomically mutate the shared state.
    ///
    /// The closure is synchronous and must not hold the lock across `.await`;
    /// the write lock is released when the closure returns.
    pub async fn update<R>(&self, f: impl FnOnce(&mut MockState) -> R) -> R {
        let mut guard = self.0.write().await;
        f(&mut guard)
    }
}

impl Deref for MockStateHandle {
    type Target = RwLock<MockState>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Mock 后端。
pub struct MockChainSdk {
    core: SdkCore,
    state: MockStateHandle,
    failures: RwLock<Vec<FailureRule>>,
    unavailable: AtomicBool,
    persistence_path: Option<PathBuf>,
    subscribers: Arc<SubscriptionTable>,
}

impl MockChainSdk {
    pub(crate) fn from_core(core: SdkCore) -> Result<Self, ChainSdkError> {
        let path = core.config.mock.state_path.clone();
        let state = match &path {
            Some(path) if path.exists() => {
                let raw = std::fs::read(path).map_err(|error| {
                    ChainSdkError::ChainUnavailable(format!(
                        "read mock state {}: {error}",
                        path.display()
                    ))
                })?;
                MockState::from_json_slice(&raw)?
            }
            _ => MockState::default(),
        };
        Ok(Self::with_state(core, state))
    }

    pub(crate) fn with_state(core: SdkCore, state: MockState) -> Self {
        Self::with_state_handle(core, MockStateHandle::new(state))
    }

    pub(crate) fn with_state_handle(core: SdkCore, state: MockStateHandle) -> Self {
        Self {
            persistence_path: core.config.mock.state_path.clone(),
            core,
            state,
            failures: RwLock::new(Vec::new()),
            unavailable: AtomicBool::new(false),
            subscribers: Arc::new(SubscriptionTable::new()),
        }
    }

    /// Shared state handle (multiple calls return the same underlying state).
    pub fn state_handle(&self) -> MockStateHandle {
        self.state.clone()
    }

    /// 当前状态快照（测试与诊断用）。
    pub async fn snapshot(&self) -> MockState {
        self.state.snapshot().await
    }

    /// 推进 Mock 内部时间并持久化。
    pub async fn advance_time(&self, seconds: u64) -> Result<(), ChainSdkError> {
        {
            let mut state = self.state.write().await;
            state.current_time = state.current_time.saturating_add(seconds);
        }
        self.persist().await
    }

    /// 追加一条故障注入规则（S5）。
    pub async fn add_failure(&self, rule: FailureRule) {
        self.failures.write().await.push(rule);
    }

    /// 设置整条链不可用（S5）。
    pub fn set_chain_unavailable(&self, unavailable: bool) {
        self.unavailable.store(unavailable, Ordering::SeqCst);
    }

    /// Persist the current shared state to `mock.state_path` (if configured).
    ///
    /// External governance writes performed through [`MockStateHandle::update`]
    /// do not emit events; callers must invoke this method to persist them.
    pub async fn persist(&self) -> Result<(), ChainSdkError> {
        let Some(path) = &self.persistence_path else {
            return Ok(());
        };
        let state = self.state.read().await.clone();
        let bytes = serde_json::to_vec_pretty(&state).map_err(|error| {
            ChainSdkError::ChainUnavailable(format!("serialize mock state: {error}"))
        })?;
        atomic_write(path, &bytes).map_err(|error| {
            ChainSdkError::ChainUnavailable(format!(
                "persist mock state {}: {error}",
                path.display()
            ))
        })
    }

    async fn emit_event(
        &self,
        event_name: &str,
        anchor_id: Option<String>,
        participant_id: Option<u32>,
        payload: serde_json::Value,
    ) {
        let block_number = self.state.read().await.current_time;
        let event = EventData {
            event_name: event_name.to_string(),
            anchor_id,
            participant_id,
            block_number: Some(block_number),
            payload,
        };
        self.subscribers.emit(&event);
    }

    async fn check_failure(&self, op: &str) -> Result<(), ChainSdkError> {
        if self.unavailable.load(Ordering::SeqCst) {
            return Err(ChainSdkError::ChainUnavailable(
                "mock chain is unavailable".into(),
            ));
        }
        let outcome = {
            let rules = self.failures.read().await;
            let mut outcome = FailureOutcome::Ok;
            for rule in rules.iter() {
                match rule {
                    FailureRule::Unavailable => {
                        outcome = FailureOutcome::Unavailable;
                        break;
                    }
                    FailureRule::Specific { op: target, error } if target == op => {
                        outcome = FailureOutcome::Specific(error.clone());
                        break;
                    }
                    FailureRule::Latency { op: target, millis } if target == op => {
                        outcome = FailureOutcome::Latency(*millis);
                        break;
                    }
                    _ => {}
                }
            }
            outcome
        };
        match outcome {
            FailureOutcome::Ok => Ok(()),
            FailureOutcome::Unavailable => Err(ChainSdkError::ChainUnavailable(
                "mock chain is unavailable".into(),
            )),
            FailureOutcome::Specific(message) => Err(ChainSdkError::NetworkError(message)),
            FailureOutcome::Latency(millis) => {
                tokio::time::sleep(Duration::from_millis(millis)).await;
                Ok(())
            }
        }
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let temp = path.with_extension(format!("tmp-{unique}"));
    {
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    std::fs::rename(&temp, path)?;
    Ok(())
}

impl MockChainSdk {
    pub(crate) fn core(&self) -> &SdkCore {
        &self.core
    }
}

impl RoleGate for MockChainSdk {
    fn role(&self) -> SdkRole {
        self.core.role
    }

    fn self_participant_id(&self) -> u32 {
        self.core.config.participant_id
    }
}

#[async_trait]
impl ChainSdk for MockChainSdk {
    fn participant_did(&self) -> &str {
        &self.core.config.participant.controller_did
    }

    async fn health_check(&self) -> Result<HealthStatus, ChainSdkError> {
        let unavailable = self.unavailable.load(Ordering::SeqCst);
        let current_time = self.state.read().await.current_time;
        Ok(HealthStatus {
            healthy: !unavailable,
            mode: "mock".into(),
            detail: unavailable.then(|| "mock chain is unavailable".to_string()),
            block_number: Some(current_time),
        })
    }

    async fn get_anchor(&self, anchor_id: &str) -> Result<AnchorRecord, ChainSdkError> {
        self.check_failure("get_anchor").await?;
        let state = self.state.read().await;
        state
            .anchors
            .get(anchor_id)
            .cloned()
            .ok_or_else(|| ChainSdkError::AnchorNotFound(anchor_id.to_string()))
    }

    async fn list_anchors(
        &self,
        anchor_type: Option<AnchorType>,
        submitter_id: Option<u32>,
    ) -> Result<Vec<AnchorRecord>, ChainSdkError> {
        self.check_failure("list_anchors").await?;
        let state = self.state.read().await;
        let mut anchors: Vec<AnchorRecord> = state
            .anchors
            .values()
            .filter(|anchor| {
                anchor_type
                    .as_ref()
                    .is_none_or(|expected| &anchor.anchor_type == expected)
            })
            .filter(|anchor| submitter_id.is_none_or(|expected| anchor.submitter_id == expected))
            .cloned()
            .collect();
        anchors.sort_by(|left, right| left.anchor_id.cmp(&right.anchor_id));
        Ok(anchors)
    }

    async fn get_participant(
        &self,
        participant_id: u32,
    ) -> Result<ParticipantRecord, ChainSdkError> {
        self.check_failure("get_participant").await?;
        let state = self.state.read().await;
        state
            .participants
            .get(&participant_id)
            .cloned()
            .ok_or(ChainSdkError::ParticipantNotFound(participant_id))
    }

    async fn get_participant_by_did(
        &self,
        did: &str,
    ) -> Result<Option<ParticipantRecord>, ChainSdkError> {
        self.check_failure("get_participant_by_did").await?;
        let state = self.state.read().await;
        Ok(state
            .did_participant
            .get(did)
            .and_then(|participant_id| state.participants.get(participant_id).cloned()))
    }

    async fn list_participants(&self) -> Result<Vec<ParticipantRecord>, ChainSdkError> {
        self.check_failure("list_participants").await?;
        let state = self.state.read().await;
        let mut participants: Vec<ParticipantRecord> =
            state.participants.values().cloned().collect();
        participants.sort_by_key(|participant| participant.participant_id);
        Ok(participants)
    }

    async fn resolve_did(&self, did: &str) -> Result<Option<DidRecord>, ChainSdkError> {
        self.check_failure("resolve_did").await?;
        Ok(self.state.read().await.dids.get(did).cloned())
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
        self.check_failure("resolve_did_web").await?;
        let port = self
            .core
            .did
            .as_ref()
            .ok_or_else(|| ChainSdkError::SignerUnavailable("did-sdk not assembled".into()))?;
        port.resolve_did_web(did).await
    }

    async fn create_anchor(
        &self,
        anchor_type: AnchorType,
        payload_hash: &str,
        valid_until: u64,
    ) -> Result<String, ChainSdkError> {
        self.ensure_issuer()?;
        self.check_failure("create_anchor").await?;

        let anchor_id = {
            let mut state = self.state.write().await;
            if !state.registered_anchor_types.contains(&anchor_type) {
                return Err(ChainSdkError::AnchorTypeNotRegistered(anchor_type));
            }
            state.next_anchor_seq = state.next_anchor_seq.saturating_add(1);
            let anchor_id = format!("anchor:{}", state.next_anchor_seq);
            let record = AnchorRecord {
                anchor_id: anchor_id.clone(),
                anchor_type: anchor_type.clone(),
                submitter_id: self.self_participant_id(),
                issuer_did: self.core.config.participant.controller_did.clone(),
                delegated_by_did: None,
                payload_hash: payload_hash.to_string(),
                revoked: false,
                created_at: state.current_time,
                valid_until,
            };
            state.anchors.insert(anchor_id.clone(), record);
            anchor_id
        };

        self.persist().await?;
        self.emit_event(
            "AnchorCreated",
            Some(anchor_id.clone()),
            Some(self.self_participant_id()),
            serde_json::json!({
                "anchor_type": anchor_type,
                "payload_hash": payload_hash,
                "valid_until": valid_until,
            }),
        )
        .await;
        Ok(anchor_id)
    }

    async fn revoke_anchor(&self, anchor_id: &str) -> Result<(), ChainSdkError> {
        self.ensure_issuer()?;
        self.check_failure("revoke_anchor").await?;

        {
            let mut state = self.state.write().await;
            let anchor = state
                .anchors
                .get_mut(anchor_id)
                .ok_or_else(|| ChainSdkError::AnchorNotFound(anchor_id.to_string()))?;
            if anchor.revoked {
                return Err(ChainSdkError::AnchorRevoked(anchor_id.to_string()));
            }
            anchor.revoked = true;
        }

        self.persist().await?;
        self.emit_event(
            "AnchorRevoked",
            Some(anchor_id.to_string()),
            Some(self.self_participant_id()),
            serde_json::json!({}),
        )
        .await;
        Ok(())
    }

    async fn renew_anchor(
        &self,
        anchor_id: &str,
        new_valid_until: u64,
    ) -> Result<(), ChainSdkError> {
        self.ensure_issuer()?;
        self.check_failure("renew_anchor").await?;

        {
            let mut state = self.state.write().await;
            let anchor = state
                .anchors
                .get_mut(anchor_id)
                .ok_or_else(|| ChainSdkError::AnchorNotFound(anchor_id.to_string()))?;
            if anchor.revoked {
                return Err(ChainSdkError::AnchorRevoked(anchor_id.to_string()));
            }
            if anchor.valid_until == 0 {
                return Err(ChainSdkError::AnchorPermanent(anchor_id.to_string()));
            }
            if new_valid_until == 0 {
                return Err(ChainSdkError::InvalidArgument(
                    "renew_anchor cannot turn a finite anchor into permanent".into(),
                ));
            }
            anchor.valid_until = new_valid_until;
        }

        self.persist().await?;
        self.emit_event(
            "AnchorRenewed",
            Some(anchor_id.to_string()),
            Some(self.self_participant_id()),
            serde_json::json!({ "valid_until": new_valid_until }),
        )
        .await;
        Ok(())
    }

    async fn update_authorized_domains(
        &self,
        participant_id: u32,
        domains: Vec<String>,
    ) -> Result<(), ChainSdkError> {
        self.ensure_self(participant_id)?;
        self.check_failure("update_authorized_domains").await?;

        {
            let mut state = self.state.write().await;
            let participant = state
                .participants
                .get_mut(&participant_id)
                .ok_or(ChainSdkError::ParticipantNotFound(participant_id))?;
            if !participant.active {
                return Err(ChainSdkError::ParticipantInactive(participant_id));
            }
            participant.authorized_domains = domains.clone();
        }

        self.persist().await?;
        self.emit_event(
            "ParticipantUpdated",
            None,
            Some(participant_id),
            serde_json::json!({ "field": "authorized_domains", "domains": domains }),
        )
        .await;
        Ok(())
    }

    async fn update_controller_did(
        &self,
        participant_id: u32,
        new_controller_did: &str,
    ) -> Result<(), ChainSdkError> {
        self.ensure_self(participant_id)?;
        self.check_failure("update_controller_did").await?;
        if new_controller_did.trim().is_empty() {
            return Err(ChainSdkError::InvalidArgument(
                "new controller_did must not be empty".into(),
            ));
        }

        {
            let mut state = self.state.write().await;
            let old_did = {
                let participant = state
                    .participants
                    .get(&participant_id)
                    .ok_or(ChainSdkError::ParticipantNotFound(participant_id))?;
                if !participant.active {
                    return Err(ChainSdkError::ParticipantInactive(participant_id));
                }
                participant.controller_did.clone()
            };
            if let Some(existing) = state.did_participant.get(new_controller_did) {
                if *existing != participant_id {
                    return Err(ChainSdkError::InvalidArgument(format!(
                        "controller_did {new_controller_did} is already bound to participant {existing}"
                    )));
                }
            }
            state.did_participant.remove(&old_did);
            state
                .did_participant
                .insert(new_controller_did.to_string(), participant_id);
            if let Some(participant) = state.participants.get_mut(&participant_id) {
                participant.controller_did = new_controller_did.to_string();
            }
        }

        self.persist().await?;
        self.emit_event(
            "ParticipantUpdated",
            None,
            Some(participant_id),
            serde_json::json!({ "field": "controller_did", "controller_did": new_controller_did }),
        )
        .await;
        Ok(())
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
        Ok(self.subscribers.register(event_name, callback))
    }
}
