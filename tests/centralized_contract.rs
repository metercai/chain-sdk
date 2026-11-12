//! S6：Centralized 端点 method + path + 查询参数契约（附录 C）。
//!
//! 覆盖：health、get_anchor（200/404/500）、list_anchors 过滤、participants
//! （含 by-did 编码）、resolve_did（200/404）。

#![allow(clippy::unwrap_used, clippy::expect_used)]

use chain_sdk::config::{
    ChainSdkConfig, ChainSdkMode, DidConfig, ParticipantConfig, SdkRole,
    build_centralized_chain_sdk,
};
use chain_sdk::{AnchorType, ChainRole, ChainSdk, ChainSdkError};
use serde_json::json;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn temp_vc_path() -> std::path::PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    std::env::temp_dir().join(format!("chain-sdk-vc-{}-{seq}", std::process::id()))
}

fn config(server: &MockServer, ttl_secs: u64) -> ChainSdkConfig {
    ChainSdkConfig {
        mode: ChainSdkMode::Centralized,
        role: SdkRole::Verifier,
        participant: ParticipantConfig {
            controller_did: "did:web:verifier.example.com".into(),
        },
        did: DidConfig {
            vc_storage_path: temp_vc_path(),
        },
        centralized_endpoint: server.uri(),
        cache_ttl_secs: ttl_secs,
        ..Default::default()
    }
}

fn anchor_json(anchor_id: &str) -> serde_json::Value {
    json!({
        "anchor_id": anchor_id,
        "anchor_type": { "kind": "EmailBinding" },
        "submitter_id": 4,
        "issuer_did": "did:web:mail.example.com:issuer",
        "delegated_by_did": null,
        "payload_hash": "blake2b:deadbeef",
        "revoked": false,
        "created_at": 1000,
        "valid_until": 0
    })
}

fn participant_json() -> serde_json::Value {
    json!({
        "participant_id": 4,
        "controller_did": "did:web:mail.example.com:issuer",
        "operator": "5GrwvaEF...",
        "chain_role": "Issuer",
        "business_types": ["EmailBinding"],
        "authorized_domains": ["mail.example.com"],
        "active": true
    })
}

#[tokio::test]
async fn health_endpoint() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "healthy": true,
            "block_number": 42
        })))
        .mount(&server)
        .await;

    let sdk = build_centralized_chain_sdk(&config(&server, 60)).unwrap();
    let health = sdk.health_check().await.unwrap();
    assert!(health.healthy);
    assert_eq!(health.mode, "centralized");
    assert_eq!(health.block_number, Some(42));
}

#[tokio::test]
async fn health_endpoint_unreachable_reports_unhealthy() {
    let server = MockServer::start().await;
    let mut cfg = config(&server, 60);
    // 指向一个未监听的端口，验证健康检查不返回 Err。
    cfg.centralized_endpoint = "http://127.0.0.1:9".into();
    let sdk = build_centralized_chain_sdk(&cfg).unwrap();
    let health = sdk.health_check().await.unwrap();
    assert!(!health.healthy);
    assert!(health.detail.is_some());
}

#[tokio::test]
async fn get_anchor_ok_and_not_found_and_server_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/anchors/a1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(anchor_json("a1")))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/anchors/a2"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let sdk = build_centralized_chain_sdk(&config(&server, 60)).unwrap();

    let anchor = sdk.get_anchor("a1").await.unwrap();
    assert_eq!(anchor.anchor_id, "a1");
    assert_eq!(anchor.submitter_id, 4);
    assert_eq!(anchor.anchor_type, AnchorType::EmailBinding);

    let error = sdk.get_anchor("missing").await.unwrap_err();
    assert!(matches!(error, ChainSdkError::AnchorNotFound(_)));

    let error = sdk.get_anchor("a2").await.unwrap_err();
    assert!(matches!(error, ChainSdkError::ChainUnavailable(_)));
}

#[tokio::test]
async fn list_anchors_passes_filters() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/anchors"))
        .and(query_param("anchor_type", "EmailBinding"))
        .and(query_param("submitter_id", "4"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([anchor_json("a1")])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/anchors"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([anchor_json("a1")])))
        .mount(&server)
        .await;

    let sdk = build_centralized_chain_sdk(&config(&server, 0)).unwrap();
    let filtered = sdk
        .list_anchors(Some(AnchorType::EmailBinding), Some(4))
        .await
        .unwrap();
    assert_eq!(filtered.len(), 1);

    let all = sdk.list_anchors(None, None).await.unwrap();
    assert_eq!(all.len(), 1);
}

#[tokio::test]
async fn participant_endpoints() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/participants/4"))
        .respond_with(ResponseTemplate::new(200).set_body_json(participant_json()))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/participants"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([participant_json()])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/participants/by-did/did:web:mail.example.com:issuer"))
        .respond_with(ResponseTemplate::new(200).set_body_json(participant_json()))
        .mount(&server)
        .await;

    let sdk = build_centralized_chain_sdk(&config(&server, 60)).unwrap();

    let participant = sdk.get_participant(4).await.unwrap();
    assert_eq!(participant.chain_role, ChainRole::Issuer);

    let by_did = sdk
        .get_participant_by_did("did:web:mail.example.com:issuer")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(by_did.participant_id, 4);

    let missing = sdk
        .get_participant_by_did("did:web:unknown.example.com")
        .await
        .unwrap();
    assert!(missing.is_none());

    let participants = sdk.list_participants().await.unwrap();
    assert_eq!(participants.len(), 1);

    let error = sdk.get_participant(9).await.unwrap_err();
    assert!(matches!(error, ChainSdkError::ParticipantNotFound(9)));
}

#[tokio::test]
async fn resolve_did_ok_and_missing() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/dids/did:web:mail.example.com:issuer"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "did": "did:web:mail.example.com:issuer",
            "doc_hash": "blake2b:aa",
            "owner": null,
            "block_number": 7
        })))
        .mount(&server)
        .await;

    let sdk = build_centralized_chain_sdk(&config(&server, 60)).unwrap();
    let record = sdk
        .resolve_did("did:web:mail.example.com:issuer")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.block_number, Some(7));

    assert!(
        sdk.resolve_did("did:web:unknown.example.com")
            .await
            .unwrap()
            .is_none()
    );
}
