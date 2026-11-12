//! S5：Mock 故障注入（延迟 / 不可用 / 指定操作错误）。

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::{Duration, Instant};

use chain_sdk::config::{ChainSdkConfig, ChainSdkMode, DidConfig, ParticipantConfig, SdkRole};
use chain_sdk::{
    ChainSdk, ChainSdkError, FailureRule, MockState, build_mock_chain_sdk,
    build_mock_chain_sdk_with_state,
};

fn temp_vc_path() -> std::path::PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    std::env::temp_dir().join(format!("chain-sdk-vc-{}-{seq}", std::process::id()))
}

fn verifier_config() -> ChainSdkConfig {
    ChainSdkConfig {
        mode: ChainSdkMode::Mock,
        role: SdkRole::Verifier,
        participant: ParticipantConfig {
            controller_did: "did:web:verifier.example.com".into(),
        },
        did: DidConfig {
            vc_storage_path: temp_vc_path(),
        },
        ..Default::default()
    }
}

fn fixture_state() -> MockState {
    MockState::from_json_str(include_str!("fixtures/mock_state.json")).unwrap()
}

#[tokio::test]
async fn unavailable_chain_rejects_reads() {
    let sdk = build_mock_chain_sdk(&verifier_config()).unwrap();
    sdk.set_chain_unavailable(true);

    let error = sdk.get_anchor("anchor:1").await.unwrap_err();
    assert!(matches!(error, ChainSdkError::ChainUnavailable(_)));
    assert!(!sdk.health_check().await.unwrap().healthy);

    sdk.set_chain_unavailable(false);
    let error = sdk.get_anchor("anchor:1").await.unwrap_err();
    assert!(matches!(error, ChainSdkError::AnchorNotFound(_)));
}

#[tokio::test]
async fn unavailable_rule_rejects_reads() {
    let sdk = build_mock_chain_sdk(&verifier_config()).unwrap();
    sdk.add_failure(FailureRule::Unavailable).await;

    let error = sdk.list_participants().await.unwrap_err();
    assert!(matches!(error, ChainSdkError::ChainUnavailable(_)));
}

#[tokio::test]
async fn specific_failure_returns_network_error() {
    let sdk = build_mock_chain_sdk(&verifier_config()).unwrap();
    sdk.add_failure(FailureRule::Specific {
        op: "get_anchor".into(),
        error: "boom".into(),
    })
    .await;

    let error = sdk.get_anchor("anchor:1").await.unwrap_err();
    match error {
        ChainSdkError::NetworkError(message) => assert_eq!(message, "boom"),
        other => panic!("unexpected error: {other:?}"),
    }

    // 其他操作不受影响。
    assert!(sdk.list_participants().await.unwrap().is_empty());
}

#[tokio::test]
async fn latency_rule_delays_matching_operation() {
    let sdk = build_mock_chain_sdk_with_state(&verifier_config(), fixture_state()).unwrap();
    sdk.add_failure(FailureRule::Latency {
        op: "get_anchor".into(),
        millis: 60,
    })
    .await;

    let started = Instant::now();
    let _ = sdk.get_anchor("anchor:1").await;
    assert!(
        started.elapsed() >= Duration::from_millis(50),
        "elapsed: {:?}",
        started.elapsed()
    );

    // 不匹配的操作不延迟。
    let started = Instant::now();
    sdk.list_participants().await.unwrap();
    assert!(started.elapsed() < Duration::from_millis(40));
}
