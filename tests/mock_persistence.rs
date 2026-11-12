//! S4：Mock 状态预置与 JSON 持久化。
//!
//! 覆盖：
//! - fixtures 预置后读得到（锚定、参与方双向索引、过滤）；
//! - `mock.state_path` 存在时启动即加载；
//! - 状态变更（`advance_time`）后原子写 JSON，重建实例后仍在。

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use chain_sdk::config::{ChainSdkConfig, ChainSdkMode, MockConfig, SdkRole};
use chain_sdk::{
    AnchorType, ChainRole, ChainSdk, MockState, build_mock_chain_sdk,
    build_mock_chain_sdk_with_state,
};

fn verifier_config(state_path: Option<PathBuf>) -> ChainSdkConfig {
    ChainSdkConfig {
        mode: ChainSdkMode::Mock,
        role: SdkRole::Verifier,
        mock: MockConfig { state_path },
        ..Default::default()
    }
}

fn fixture_state() -> MockState {
    MockState::from_json_str(include_str!("fixtures/mock_state.json")).unwrap()
}

#[tokio::test]
async fn preloaded_fixture_is_readable() {
    let sdk = build_mock_chain_sdk_with_state(&verifier_config(None), fixture_state()).unwrap();

    let anchor = sdk.get_anchor("anchor:1").await.unwrap();
    assert_eq!(anchor.issuer_did, "did:web:mail.example.com:issuer");
    assert_eq!(anchor.submitter_id, 4);
    assert!(anchor.delegated_by_did.is_none());

    let delegated = sdk.get_anchor("anchor:delegated-1").await.unwrap();
    assert_eq!(delegated.issuer_did, "did:key:zA");
    assert_eq!(delegated.submitter_id, 7);
    assert_eq!(
        delegated.delegated_by_did.as_deref(),
        Some("did:web:proxy.example.com")
    );

    let participant = sdk.get_participant(4).await.unwrap();
    assert_eq!(participant.chain_role, ChainRole::Issuer);
    assert_eq!(participant.authorized_domains, vec!["mail.example.com"]);

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

    let issued_vc = sdk
        .list_anchors(Some(AnchorType::IssuedVc), None)
        .await
        .unwrap();
    assert_eq!(issued_vc.len(), 1);
    assert_eq!(issued_vc[0].anchor_id, "anchor:delegated-1");

    let by_submitter = sdk.list_anchors(None, Some(7)).await.unwrap();
    assert_eq!(by_submitter.len(), 1);

    let participants = sdk.list_participants().await.unwrap();
    assert_eq!(
        participants
            .iter()
            .map(|item| item.participant_id)
            .collect::<Vec<_>>(),
        vec![4, 7]
    );
}

#[tokio::test]
async fn loads_state_from_configured_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mock_state.json");
    std::fs::write(&path, include_str!("fixtures/mock_state.json")).unwrap();

    let sdk = build_mock_chain_sdk(&verifier_config(Some(path))).unwrap();
    assert_eq!(sdk.snapshot().await.current_time, 1000);
    assert_eq!(sdk.list_participants().await.unwrap().len(), 2);
}

#[tokio::test]
async fn state_is_persisted_after_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    let config = verifier_config(Some(path.clone()));

    let sdk = build_mock_chain_sdk(&config).unwrap();
    sdk.advance_time(42).await.unwrap();
    assert_eq!(sdk.snapshot().await.current_time, 42);

    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(raw.contains("\"current_time\": 42"), "{raw}");

    let reloaded = build_mock_chain_sdk(&config).unwrap();
    assert_eq!(reloaded.snapshot().await.current_time, 42);
}

#[tokio::test]
async fn health_check_reports_mock_mode() {
    let sdk = build_mock_chain_sdk(&verifier_config(None)).unwrap();
    let health = sdk.health_check().await.unwrap();
    assert!(health.healthy);
    assert_eq!(health.mode, "mock");
    assert_eq!(health.block_number, Some(0));
}
