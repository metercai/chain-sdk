//! CS-03: structured Centralized error bodies (409 semantics, codes, fallback).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use chain_sdk::config::{
    ChainSdkConfig, ChainSdkMode, DidConfig, KeyVaultConfig, KeyVaultKeys, ParticipantConfig,
    SdkRole,
};
use chain_sdk::{
    AnchorType, CentralizedChainSdk, ChainSdk, ChainSdkError,
    build_centralized_chain_sdk_with_vault,
};
use keyvault_core::{KeyAlgorithm, KeyNamespace, KeyVault, MemoryKeystore};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn temp_vc_path() -> std::path::PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    std::env::temp_dir().join(format!("chain-sdk-vc-{}-{seq}", std::process::id()))
}

async fn sdk(server: &MockServer) -> Arc<CentralizedChainSdk> {
    let vault: Arc<dyn KeyVault> = Arc::new(MemoryKeystore::new());
    let chain_key = vault
        .generate(KeyNamespace::ChainAccount, KeyAlgorithm::Sr25519)
        .await
        .unwrap();
    let did_key = vault
        .generate(KeyNamespace::ParticipantDid, KeyAlgorithm::Ed25519)
        .await
        .unwrap();

    let config = ChainSdkConfig {
        mode: ChainSdkMode::Centralized,
        role: SdkRole::Issuer,
        participant_id: 4,
        participant: ParticipantConfig {
            controller_did: "did:web:mail.example.com:issuer".into(),
        },
        did: DidConfig {
            vc_storage_path: temp_vc_path(),
        },
        key_vault: KeyVaultConfig {
            scheme: "memory-keystore".into(),
            keys: KeyVaultKeys {
                operator: chain_key.0,
                participant_did: did_key.0,
            },
            ..Default::default()
        },
        centralized_endpoint: server.uri(),
        cache_ttl_secs: 0,
        ..Default::default()
    };
    build_centralized_chain_sdk_with_vault(&config, vault).unwrap()
}

fn error_body(code: &str) -> serde_json::Value {
    json!({ "code": code, "message": format!("{code} message"), "request_id": "req-1" })
}

#[tokio::test]
async fn renew_permanent_maps_to_anchor_permanent() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/anchors/a1/renew"))
        .respond_with(ResponseTemplate::new(409).set_body_json(error_body("ANCHOR_PERMANENT")))
        .mount(&server)
        .await;

    let error = sdk(&server)
        .await
        .renew_anchor("a1", 100)
        .await
        .unwrap_err();
    assert!(matches!(error, ChainSdkError::AnchorPermanent(id) if id == "a1"));
}

#[tokio::test]
async fn renew_revoked_maps_to_anchor_revoked() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/anchors/a1/renew"))
        .respond_with(ResponseTemplate::new(409).set_body_json(error_body("ANCHOR_REVOKED")))
        .mount(&server)
        .await;

    let error = sdk(&server)
        .await
        .renew_anchor("a1", 100)
        .await
        .unwrap_err();
    assert!(matches!(error, ChainSdkError::AnchorRevoked(id) if id == "a1"));
}

#[tokio::test]
async fn plain_text_conflict_falls_back_to_invalid_argument() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/anchors/a1/renew"))
        .respond_with(ResponseTemplate::new(409).set_body_string("conflict"))
        .mount(&server)
        .await;

    let error = sdk(&server)
        .await
        .renew_anchor("a1", 100)
        .await
        .unwrap_err();
    assert!(matches!(error, ChainSdkError::InvalidArgument(_)));
}

#[tokio::test]
async fn internal_error_maps_to_chain_unavailable() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/anchors/a1/renew"))
        .respond_with(ResponseTemplate::new(500).set_body_json(error_body("INTERNAL")))
        .mount(&server)
        .await;

    let error = sdk(&server)
        .await
        .renew_anchor("a1", 100)
        .await
        .unwrap_err();
    assert!(matches!(error, ChainSdkError::ChainUnavailable(_)));
}

#[tokio::test]
async fn unauthorized_code_maps_to_unauthorized() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/participants/4/domains"))
        .respond_with(ResponseTemplate::new(403).set_body_json(error_body("INVALID_SIGNATURE")))
        .mount(&server)
        .await;

    let error = sdk(&server)
        .await
        .update_authorized_domains(4, vec!["example.com".into()])
        .await
        .unwrap_err();
    assert!(matches!(error, ChainSdkError::Unauthorized(_)));
}

#[tokio::test]
async fn inactive_participant_keeps_participant_id() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/participants/4/domains"))
        .respond_with(ResponseTemplate::new(409).set_body_json(error_body("PARTICIPANT_INACTIVE")))
        .mount(&server)
        .await;

    let error = sdk(&server)
        .await
        .update_authorized_domains(4, vec!["example.com".into()])
        .await
        .unwrap_err();
    assert!(matches!(error, ChainSdkError::ParticipantInactive(4)));
}

#[tokio::test]
async fn unregistered_anchor_type_uses_details() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/anchors"))
        .respond_with(ResponseTemplate::new(409).set_body_json(json!({
            "code": "ANCHOR_TYPE_NOT_REGISTERED",
            "message": "not registered",
            "request_id": "req-2",
            "details": { "anchor_type": { "kind": "Custom", "name": "Contract" } }
        })))
        .mount(&server)
        .await;

    let error = sdk(&server)
        .await
        .create_anchor(AnchorType::Custom("Contract".into()), "blake2b:aa", 0)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ChainSdkError::AnchorTypeNotRegistered(AnchorType::Custom(name)) if name == "Contract"
    ));
}

#[tokio::test]
async fn oversized_error_body_is_bounded_and_falls_back() {
    let server = MockServer::start().await;
    let huge = "x".repeat(32 * 1024);
    Mock::given(method("POST"))
        .and(path("/anchors/a1/revoke"))
        .respond_with(ResponseTemplate::new(409).set_body_string(huge))
        .mount(&server)
        .await;

    let error = sdk(&server).await.revoke_anchor("a1").await.unwrap_err();
    assert!(matches!(error, ChainSdkError::InvalidArgument(_)));
}

#[tokio::test]
async fn revoke_anchor_revoked_code_maps() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/anchors/a1/revoke"))
        .respond_with(ResponseTemplate::new(409).set_body_json(error_body("ANCHOR_REVOKED")))
        .mount(&server)
        .await;

    let error = sdk(&server).await.revoke_anchor("a1").await.unwrap_err();
    assert!(matches!(error, ChainSdkError::AnchorRevoked(id) if id == "a1"));
}
