//! S11：事件订阅（Mock 内存 emit；Centralized WebSocket）。
//!
//! Chain 后端的事件订阅在 P10 用 `subxt::events().subscribe()` 接入。

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use chain_sdk::config::build_chain_sdk;
use chain_sdk::config::{
    ChainSdkConfig, ChainSdkMode, DidConfig, KeyVaultConfig, ParticipantConfig, SdkRole,
    build_centralized_chain_sdk, build_mock_chain_sdk_with_state,
};
use chain_sdk::{AnchorType, ChainSdk, EventData, MockState};
use futures::SinkExt;
use tokio_tungstenite::tungstenite::Message;

fn temp_vc_path() -> std::path::PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    std::env::temp_dir().join(format!("chain-sdk-vc-{}-{seq}", std::process::id()))
}

fn mock_config() -> ChainSdkConfig {
    ChainSdkConfig {
        mode: ChainSdkMode::Mock,
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
            ..Default::default()
        },
        ..Default::default()
    }
}

fn fixture_state() -> MockState {
    MockState::from_json_str(include_str!("fixtures/mock_state.json")).unwrap()
}

#[tokio::test]
async fn mock_events_are_delivered_and_unsubscribe_stops() {
    let sdk = build_mock_chain_sdk_with_state(&mock_config(), fixture_state()).unwrap();
    let seen = Arc::new(Mutex::new(Vec::<EventData>::new()));

    let unsubscribe = sdk
        .subscribe_event(
            "AnchorCreated",
            Box::new({
                let seen = Arc::clone(&seen);
                move |event| seen.lock().unwrap().push(event)
            }),
        )
        .unwrap();

    let anchor_id = sdk
        .create_anchor(AnchorType::EmailBinding, "blake2b:aa", 0)
        .await
        .unwrap();

    let events = seen.lock().unwrap().clone();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_name, "AnchorCreated");
    assert_eq!(events[0].anchor_id.as_deref(), Some(anchor_id.as_str()));
    assert_eq!(events[0].participant_id, Some(4));
    assert_eq!(events[0].block_number, Some(1000));

    unsubscribe();
    sdk.create_anchor(AnchorType::EmailBinding, "blake2b:bb", 0)
        .await
        .unwrap();
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn mock_wildcard_receives_participant_updates() {
    let sdk = build_mock_chain_sdk_with_state(&mock_config(), fixture_state()).unwrap();
    let seen = Arc::new(Mutex::new(Vec::<EventData>::new()));
    let _unsubscribe = sdk
        .subscribe_event(
            "*",
            Box::new({
                let seen = Arc::clone(&seen);
                move |event| seen.lock().unwrap().push(event)
            }),
        )
        .unwrap();

    sdk.update_authorized_domains(4, vec!["example.com".into()])
        .await
        .unwrap();

    let events = seen.lock().unwrap().clone();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_name, "ParticipantUpdated");
    assert_eq!(events[0].participant_id, Some(4));
}

async fn spawn_ws_server(event: serde_json::Value) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let Ok((stream, _)) = listener.accept().await else {
            return;
        };
        let Ok(mut socket) = tokio_tungstenite::accept_async(stream).await else {
            return;
        };
        let _ = socket.send(Message::Text(event.to_string())).await;
        tokio::time::sleep(Duration::from_secs(5)).await;
    });
    (format!("http://{address}"), handle)
}

#[tokio::test]
async fn centralized_events_are_delivered_over_websocket() {
    let payload = serde_json::json!({
        "event_name": "AnchorCreated",
        "anchor_id": "a1",
        "participant_id": 4,
        "block_number": 9,
        "payload": { "anchor_type": { "kind": "EmailBinding" } }
    });
    let (endpoint, server) = spawn_ws_server(payload).await;

    let config = ChainSdkConfig {
        mode: ChainSdkMode::Centralized,
        role: SdkRole::Verifier,
        participant: ParticipantConfig {
            controller_did: "did:web:verifier.example.com".into(),
        },
        did: DidConfig {
            vc_storage_path: temp_vc_path(),
        },
        centralized_endpoint: endpoint,
        ..Default::default()
    };
    let sdk = build_centralized_chain_sdk(&config).unwrap();
    let seen = Arc::new(Mutex::new(Vec::<EventData>::new()));
    let _unsubscribe = sdk
        .subscribe_event(
            "AnchorCreated",
            Box::new({
                let seen = Arc::clone(&seen);
                move |event| seen.lock().unwrap().push(event)
            }),
        )
        .unwrap();

    let mut delivered = false;
    for _ in 0..60 {
        if !seen.lock().unwrap().is_empty() {
            delivered = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    server.abort();
    assert!(delivered, "expected a websocket event within 3s");

    let events = seen.lock().unwrap().clone();
    assert_eq!(events[0].event_name, "AnchorCreated");
    assert_eq!(events[0].anchor_id.as_deref(), Some("a1"));
    assert_eq!(events[0].block_number, Some(9));
}

#[tokio::test]
async fn chain_subscription_registers_without_node() {
    let config = ChainSdkConfig {
        mode: ChainSdkMode::Chain,
        role: SdkRole::Verifier,
        participant: ParticipantConfig {
            controller_did: "did:web:verifier.example.com".into(),
        },
        did: DidConfig {
            vc_storage_path: temp_vc_path(),
        },
        chain_endpoint: "ws://127.0.0.1:1".into(),
        ..Default::default()
    };
    let sdk = build_chain_sdk(&config).unwrap();
    // 订阅注册是同步的；连接失败在后台重试，不影响注册。
    let unsubscribe = sdk
        .subscribe_event("AnchorCreated", Box::new(|_event| {}))
        .unwrap();
    unsubscribe();
}
