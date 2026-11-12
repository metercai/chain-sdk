//! S8：`get_anchor` TTL 缓存（命中、过期穿透、关闭缓存）。

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use chain_sdk::ChainSdk;
use chain_sdk::config::{
    ChainSdkConfig, ChainSdkMode, DidConfig, ParticipantConfig, SdkRole,
    build_centralized_chain_sdk,
};
use serde_json::json;
use wiremock::matchers::{method, path};
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

fn anchor_json() -> serde_json::Value {
    json!({
        "anchor_id": "a1",
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

#[tokio::test]
async fn cache_hit_avoids_second_request() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/anchors/a1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(anchor_json()))
        .expect(1)
        .mount(&server)
        .await;

    let sdk = build_centralized_chain_sdk(&config(&server, 60)).unwrap();
    let first = sdk.get_anchor("a1").await.unwrap();
    let second = sdk.get_anchor("a1").await.unwrap();
    assert_eq!(first.anchor_id, second.anchor_id);

    server.verify().await;
}

#[tokio::test]
async fn cache_expires_after_ttl() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/anchors/a1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(anchor_json()))
        .expect(2)
        .mount(&server)
        .await;

    let sdk = build_centralized_chain_sdk(&config(&server, 1)).unwrap();
    sdk.get_anchor("a1").await.unwrap();
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    sdk.get_anchor("a1").await.unwrap();

    server.verify().await;
}

#[tokio::test]
async fn cache_disabled_with_zero_ttl() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/anchors/a1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(anchor_json()))
        .expect(2)
        .mount(&server)
        .await;

    let sdk = build_centralized_chain_sdk(&config(&server, 0)).unwrap();
    sdk.get_anchor("a1").await.unwrap();
    sdk.get_anchor("a1").await.unwrap();

    server.verify().await;
}
