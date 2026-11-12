//! `Centralized` 后端：HTTP 只读 + TTL 缓存（P5）；写请求签名在 P6。
//!
//! 端点契约见 `docs/implementation-plan.md` 附录 C。只对 `get_anchor` 启用 TTL
//! 缓存；`cache_ttl_secs = 0` 关闭缓存。

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use async_lock::RwLock;
use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use reqwest::{Client, StatusCode, Url};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::events::SubscriptionTable;
use crate::role_gate::RoleGate;
use crate::{
    AnchorRecord, AnchorType, AssertionPurpose, ChainSdk, ChainSdkError, DidDocument, DidRecord,
    Ed25519PublicKey, Ed25519Signature, EventData, HealthStatus, ParticipantRecord, SdkCore,
    SdkRole, Unsubscribe,
};

#[derive(Debug, Clone)]
struct CacheEntry {
    value: AnchorRecord,
    inserted: Instant,
}

#[derive(Debug, Deserialize)]
struct HealthPayload {
    healthy: bool,
    #[serde(default)]
    block_number: Option<u64>,
    #[serde(default)]
    detail: Option<String>,
}

/// 中心化模拟链后端（HTTP）。
pub struct CentralizedChainSdk {
    core: SdkCore,
    http: Client,
    endpoint: String,
    ttl_secs: u64,
    anchors: RwLock<HashMap<String, CacheEntry>>,
    subscribers: Arc<SubscriptionTable>,
    ws_started: AtomicBool,
}

impl CentralizedChainSdk {
    pub(crate) fn new(core: SdkCore) -> Result<Self, ChainSdkError> {
        let http = Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent(concat!("chain-sdk/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| ChainSdkError::ConfigError(format!("build HTTP client: {error}")))?;
        Ok(Self {
            endpoint: core.config.centralized_endpoint.clone(),
            ttl_secs: core.config.cache_ttl_secs,
            core,
            http,
            anchors: RwLock::new(HashMap::new()),
            subscribers: Arc::new(SubscriptionTable::new()),
            ws_started: AtomicBool::new(false),
        })
    }

    fn url(&self, segments: &[&str]) -> Result<Url, ChainSdkError> {
        let mut url = Url::parse(&self.endpoint).map_err(|error| {
            ChainSdkError::ConfigError(format!("invalid centralized_endpoint: {error}"))
        })?;
        {
            let mut path = url.path_segments_mut().map_err(|()| {
                ChainSdkError::ConfigError(
                    "centralized_endpoint cannot be used as a base URL".into(),
                )
            })?;
            path.pop_if_empty();
            for segment in segments {
                path.push(segment);
            }
        }
        Ok(url)
    }

    async fn send(&self, url: Url, op: &str) -> Result<reqwest::Response, ChainSdkError> {
        self.http
            .get(url)
            .send()
            .await
            .map_err(|error| ChainSdkError::NetworkError(format!("{op}: {error}")))
    }

    async fn fetch_optional<T: DeserializeOwned>(
        &self,
        url: Url,
        op: &str,
    ) -> Result<Option<T>, ChainSdkError> {
        let response = self.send(url, op).await?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            return Err(status_error(response.status(), op));
        }
        response
            .json::<T>()
            .await
            .map(Some)
            .map_err(|error| ChainSdkError::NetworkError(format!("{op}: invalid JSON: {error}")))
    }

    async fn fetch_required<T: DeserializeOwned>(
        &self,
        url: Url,
        op: &str,
    ) -> Result<T, ChainSdkError> {
        let response = self.send(url, op).await?;
        if !response.status().is_success() {
            return Err(status_error(response.status(), op));
        }
        response
            .json::<T>()
            .await
            .map_err(|error| ChainSdkError::NetworkError(format!("{op}: invalid JSON: {error}")))
    }

    fn cache_fresh(&self, entry: &CacheEntry) -> bool {
        entry.inserted.elapsed() < Duration::from_secs(self.ttl_secs)
    }

    async fn invalidate_anchor(&self, anchor_id: &str) {
        self.anchors.write().await.remove(anchor_id);
    }

    /// 发送签名写请求（调用方必须先完成角色门禁）。
    ///
    /// 待签名字节 = `op || "\n" || canonical_json(body)`；签名头见附录 C.2。
    async fn signed_post<T: Serialize>(
        &self,
        op: &str,
        segments: &[&str],
        body: &T,
    ) -> Result<reqwest::Response, ChainSdkError> {
        let vault =
            self.core.vault.as_ref().ok_or_else(|| {
                ChainSdkError::SignerUnavailable("issuer requires KeyVault".into())
            })?;
        let keys = self
            .core
            .keys
            .as_ref()
            .ok_or_else(|| ChainSdkError::SignerUnavailable("keys not loaded".into()))?;

        let canonical = canonical_json(body)?;
        let mut payload = Vec::with_capacity(op.len() + 1 + canonical.len());
        payload.extend_from_slice(op.as_bytes());
        payload.push(b'\n');
        payload.extend_from_slice(&canonical);

        // S14：经唯一入口签名，强制 chain-account/ namespace。
        let signature =
            crate::keys::sign_extrinsic(vault.as_ref(), &keys.chain_account, &payload).await?;

        let url = self.url(segments)?;
        self.http
            .post(url)
            .header("x-chain-signer", &keys.chain_account.0)
            .header("x-chain-signature", hex::encode(&signature.0))
            .header("x-chain-op", op)
            .json(body)
            .send()
            .await
            .map_err(|error| ChainSdkError::NetworkError(format!("{op}: {error}")))
    }
}

/// 写请求的 canonical JSON：对象键递归升序、紧凑、拒绝浮点（附录 C.3）。
fn canonical_json<T: Serialize>(value: &T) -> Result<Vec<u8>, ChainSdkError> {
    let value = serde_json::to_value(value)
        .map_err(|error| ChainSdkError::InvalidArgument(format!("canonical JSON: {error}")))?;
    let canonical = canonicalize(value)?;
    serde_json::to_vec(&canonical)
        .map_err(|error| ChainSdkError::InvalidArgument(format!("canonical JSON: {error}")))
}

fn canonicalize(value: Value) -> Result<Value, ChainSdkError> {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(String, Value)> = map.into_iter().collect();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            let mut sorted = serde_json::Map::new();
            for (key, value) in entries {
                sorted.insert(key, canonicalize(value)?);
            }
            Ok(Value::Object(sorted))
        }
        Value::Array(items) => {
            let mut canonical = Vec::with_capacity(items.len());
            for item in items {
                canonical.push(canonicalize(item)?);
            }
            Ok(Value::Array(canonical))
        }
        Value::Number(number) if number.is_f64() => Err(ChainSdkError::InvalidArgument(
            "canonical JSON rejects floating point numbers".into(),
        )),
        other => Ok(other),
    }
}

#[derive(Serialize)]
struct CreateAnchorBody<'a> {
    anchor_type: &'a AnchorType,
    payload_hash: &'a str,
    valid_until: u64,
}

#[derive(Deserialize)]
struct CreateAnchorResponse {
    anchor_id: String,
}

async fn parse_json<T: DeserializeOwned>(
    response: reqwest::Response,
    op: &str,
) -> Result<T, ChainSdkError> {
    if !response.status().is_success() {
        return Err(status_error(response.status(), op));
    }
    response
        .json::<T>()
        .await
        .map_err(|error| ChainSdkError::NetworkError(format!("{op}: invalid JSON: {error}")))
}

async fn ensure_success(response: reqwest::Response, op: &str) -> Result<(), ChainSdkError> {
    if response.status().is_success() {
        Ok(())
    } else {
        Err(status_error(response.status(), op))
    }
}

fn status_error(status: StatusCode, op: &str) -> ChainSdkError {
    match status.as_u16() {
        401 | 403 => ChainSdkError::Unauthorized(format!("{op}: HTTP {status}")),
        409 => ChainSdkError::InvalidArgument(format!("{op}: conflict (HTTP {status})")),
        _ if status.is_server_error() => {
            ChainSdkError::ChainUnavailable(format!("{op}: HTTP {status}"))
        }
        _ => ChainSdkError::NetworkError(format!("{op}: HTTP {status}")),
    }
}

impl RoleGate for CentralizedChainSdk {
    fn role(&self) -> SdkRole {
        self.core.role
    }

    fn self_participant_id(&self) -> u32 {
        self.core.config.participant_id
    }
}

#[async_trait]
impl ChainSdk for CentralizedChainSdk {
    fn participant_did(&self) -> &str {
        &self.core.config.participant.controller_did
    }

    async fn health_check(&self) -> Result<HealthStatus, ChainSdkError> {
        let url = self.url(&["health"])?;
        match self
            .fetch_required::<HealthPayload>(url, "health_check")
            .await
        {
            Ok(payload) => Ok(HealthStatus {
                healthy: payload.healthy,
                mode: "centralized".into(),
                detail: payload.detail,
                block_number: payload.block_number,
            }),
            Err(error) => Ok(HealthStatus {
                healthy: false,
                mode: "centralized".into(),
                detail: Some(error.to_string()),
                block_number: None,
            }),
        }
    }

    async fn get_anchor(&self, anchor_id: &str) -> Result<AnchorRecord, ChainSdkError> {
        if self.ttl_secs > 0 {
            let cache = self.anchors.read().await;
            if let Some(entry) = cache.get(anchor_id) {
                if self.cache_fresh(entry) {
                    return Ok(entry.value.clone());
                }
            }
        }

        let url = self.url(&["anchors", anchor_id])?;
        let anchor = self
            .fetch_optional::<AnchorRecord>(url, "get_anchor")
            .await?
            .ok_or_else(|| ChainSdkError::AnchorNotFound(anchor_id.to_string()))?;

        if self.ttl_secs > 0 {
            self.anchors.write().await.insert(
                anchor_id.to_string(),
                CacheEntry {
                    value: anchor.clone(),
                    inserted: Instant::now(),
                },
            );
        }
        Ok(anchor)
    }

    async fn list_anchors(
        &self,
        anchor_type: Option<AnchorType>,
        submitter_id: Option<u32>,
    ) -> Result<Vec<AnchorRecord>, ChainSdkError> {
        let mut url = self.url(&["anchors"])?;
        {
            let mut query = url.query_pairs_mut();
            if let Some(anchor_type) = anchor_type.as_ref() {
                query.append_pair("anchor_type", anchor_type.as_str());
            }
            if let Some(submitter_id) = submitter_id {
                query.append_pair("submitter_id", &submitter_id.to_string());
            }
        }
        self.fetch_required::<Vec<AnchorRecord>>(url, "list_anchors")
            .await
    }

    async fn get_participant(
        &self,
        participant_id: u32,
    ) -> Result<ParticipantRecord, ChainSdkError> {
        let url = self.url(&["participants", &participant_id.to_string()])?;
        self.fetch_optional::<ParticipantRecord>(url, "get_participant")
            .await?
            .ok_or(ChainSdkError::ParticipantNotFound(participant_id))
    }

    async fn get_participant_by_did(
        &self,
        did: &str,
    ) -> Result<Option<ParticipantRecord>, ChainSdkError> {
        let url = self.url(&["participants", "by-did", did])?;
        self.fetch_optional::<ParticipantRecord>(url, "get_participant_by_did")
            .await
    }

    async fn list_participants(&self) -> Result<Vec<ParticipantRecord>, ChainSdkError> {
        let url = self.url(&["participants"])?;
        self.fetch_required::<Vec<ParticipantRecord>>(url, "list_participants")
            .await
    }

    async fn resolve_did(&self, did: &str) -> Result<Option<DidRecord>, ChainSdkError> {
        let url = self.url(&["dids", did])?;
        self.fetch_optional::<DidRecord>(url, "resolve_did").await
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
        anchor_type: AnchorType,
        payload_hash: &str,
        valid_until: u64,
    ) -> Result<String, ChainSdkError> {
        self.ensure_issuer()?;
        let body = CreateAnchorBody {
            anchor_type: &anchor_type,
            payload_hash,
            valid_until,
        };
        let response = self
            .signed_post("create_anchor", &["anchors"], &body)
            .await?;
        let created: CreateAnchorResponse = parse_json(response, "create_anchor").await?;
        Ok(created.anchor_id)
    }

    async fn revoke_anchor(&self, anchor_id: &str) -> Result<(), ChainSdkError> {
        self.ensure_issuer()?;
        let response = self
            .signed_post(
                "revoke_anchor",
                &["anchors", anchor_id, "revoke"],
                &serde_json::json!({}),
            )
            .await?;
        if response.status() == StatusCode::CONFLICT {
            return Err(ChainSdkError::AnchorRevoked(anchor_id.to_string()));
        }
        ensure_success(response, "revoke_anchor").await?;
        self.invalidate_anchor(anchor_id).await;
        Ok(())
    }

    async fn renew_anchor(
        &self,
        anchor_id: &str,
        new_valid_until: u64,
    ) -> Result<(), ChainSdkError> {
        self.ensure_issuer()?;
        let response = self
            .signed_post(
                "renew_anchor",
                &["anchors", anchor_id, "renew"],
                &serde_json::json!({ "valid_until": new_valid_until }),
            )
            .await?;
        if response.status() == StatusCode::CONFLICT {
            return Err(ChainSdkError::AnchorRevoked(anchor_id.to_string()));
        }
        ensure_success(response, "renew_anchor").await?;
        self.invalidate_anchor(anchor_id).await;
        Ok(())
    }

    async fn update_authorized_domains(
        &self,
        participant_id: u32,
        domains: Vec<String>,
    ) -> Result<(), ChainSdkError> {
        self.ensure_self(participant_id)?;
        let response = self
            .signed_post(
                "update_authorized_domains",
                &["participants", &participant_id.to_string(), "domains"],
                &serde_json::json!({ "domains": domains }),
            )
            .await?;
        ensure_success(response, "update_authorized_domains").await
    }

    async fn update_controller_did(
        &self,
        participant_id: u32,
        new_controller_did: &str,
    ) -> Result<(), ChainSdkError> {
        self.ensure_self(participant_id)?;
        let response = self
            .signed_post(
                "update_controller_did",
                &[
                    "participants",
                    &participant_id.to_string(),
                    "controller-did",
                ],
                &serde_json::json!({ "controller_did": new_controller_did }),
            )
            .await?;
        ensure_success(response, "update_controller_did").await
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
        if !self.ws_started.swap(true, Ordering::SeqCst) {
            start_event_stream(
                self.core.config.centralized_endpoint.clone(),
                Arc::clone(&self.subscribers),
            );
        }
        Ok(handle)
    }
}

fn websocket_url(endpoint: &str) -> String {
    let trimmed = endpoint.trim_end_matches('/');
    if let Some(rest) = trimmed.strip_prefix("https://") {
        format!("wss://{rest}/events")
    } else if let Some(rest) = trimmed.strip_prefix("http://") {
        format!("ws://{rest}/events")
    } else if trimmed.starts_with("ws://") || trimmed.starts_with("wss://") {
        format!("{trimmed}/events")
    } else {
        format!("ws://{trimmed}/events")
    }
}

/// 单连接 + 重连的事件流（POC）。
///
/// `Unsubscribe` 只移除回调；连接保持到进程结束（或服务端关闭）。
async fn run_event_stream(endpoint: String, table: Arc<SubscriptionTable>) {
    loop {
        let url = websocket_url(&endpoint);
        match tokio_tungstenite::connect_async(&url).await {
            Ok((mut socket, _)) => {
                while let Some(message) = socket.next().await {
                    match message {
                        Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => {
                            match serde_json::from_str::<EventData>(&text) {
                                Ok(event) => table.emit(&event),
                                Err(error) => {
                                    tracing::warn!(%error, "ignore malformed event frame");
                                }
                            }
                        }
                        Ok(tokio_tungstenite::tungstenite::Message::Ping(payload)) => {
                            let _ = socket
                                .send(tokio_tungstenite::tungstenite::Message::Pong(payload))
                                .await;
                        }
                        Ok(tokio_tungstenite::tungstenite::Message::Close(_)) | Err(_) => break,
                        _ => {}
                    }
                }
            }
            Err(error) => {
                tracing::warn!(%error, %url, "centralized event socket unavailable; retrying");
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::canonical_json;

    #[test]
    fn canonical_json_sorts_keys_and_rejects_floats() {
        let bytes =
            canonical_json(&serde_json::json!({ "b": 1, "a": { "d": 4, "c": 3 } })).unwrap();
        assert_eq!(bytes, br#"{"a":{"c":3,"d":4},"b":1}"#);

        assert!(canonical_json(&serde_json::json!({ "x": 1.5 })).is_err());
    }
}
