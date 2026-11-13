//! CS-02: `MockStateHandle` shared state with persistence.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use chain_sdk::config::{
    ChainSdkConfig, ChainSdkMode, DidConfig, KeyVaultConfig, MockConfig, ParticipantConfig, SdkRole,
};
use chain_sdk::{
    AnchorType, BusinessType, ChainRole, ChainSdk, MockState, MockStateHandle, ParticipantRecord,
    build_mock_chain_sdk, build_mock_chain_sdk_with_state_handle,
};

const SELF_DID: &str = "did:web:mail.example.com:issuer";

fn temp_vc_path() -> PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    std::env::temp_dir().join(format!("chain-sdk-vc-{}-{seq}", std::process::id()))
}

fn config(state_path: Option<PathBuf>) -> ChainSdkConfig {
    ChainSdkConfig {
        mode: ChainSdkMode::Mock,
        role: SdkRole::Issuer,
        participant_id: 4,
        participant: ParticipantConfig {
            controller_did: SELF_DID.into(),
        },
        did: DidConfig {
            vc_storage_path: temp_vc_path(),
        },
        mock: MockConfig { state_path },
        key_vault: KeyVaultConfig {
            scheme: "memory-keystore".into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn participant(participant_id: u32, did: &str) -> ParticipantRecord {
    ParticipantRecord {
        participant_id,
        controller_did: did.into(),
        operator: "5GrwvaEF...".into(),
        chain_role: ChainRole::Issuer,
        business_types: vec![BusinessType::EmailBinding],
        authorized_domains: vec!["mail.example.com".into()],
        active: true,
    }
}

async fn seed(handle: &MockStateHandle) {
    handle
        .update(|state| {
            state.current_time = 1000;
            state.participants.insert(4, participant(4, SELF_DID));
            state.did_participant.insert(SELF_DID.into(), 4);
            state.account_participant.insert("5GrwvaEF...".into(), 4);
            state
                .registered_anchor_types
                .insert(AnchorType::EmailBinding);
        })
        .await;
}

#[tokio::test]
async fn shared_handle_makes_governance_writes_visible() {
    let handle = MockStateHandle::new(MockState::default());
    let sdk = build_mock_chain_sdk_with_state_handle(&config(None), handle.clone()).unwrap();

    assert!(matches!(
        sdk.get_participant(4).await,
        Err(chain_sdk::ChainSdkError::ParticipantNotFound(4))
    ));

    seed(&handle).await;

    let participant = sdk.get_participant(4).await.unwrap();
    assert_eq!(participant.controller_did, SELF_DID);
    assert_eq!(
        sdk.get_participant_by_did(SELF_DID)
            .await
            .unwrap()
            .unwrap()
            .participant_id,
        4
    );

    let anchor_id = sdk
        .create_anchor(AnchorType::EmailBinding, "blake2b:aa", 0)
        .await
        .unwrap();
    assert!(handle.snapshot().await.anchors.contains_key(&anchor_id));
    assert!(sdk.get_anchor(&anchor_id).await.is_ok());
}

#[tokio::test]
async fn state_handle_identity_is_shared() {
    let handle = MockStateHandle::new(MockState::default());
    let sdk = build_mock_chain_sdk_with_state_handle(&config(None), handle.clone()).unwrap();

    sdk.state_handle()
        .update(|state| state.current_time = 42)
        .await;
    assert_eq!(handle.snapshot().await.current_time, 42);
    assert_eq!(sdk.snapshot().await.current_time, 42);
}

#[tokio::test]
async fn explicit_persist_survives_rebuild() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mock-state.json");
    let cfg = config(Some(path.clone()));

    let handle = MockStateHandle::new(MockState::default());
    let sdk = build_mock_chain_sdk_with_state_handle(&cfg, handle.clone()).unwrap();
    seed(&handle).await;
    sdk.persist().await.unwrap();
    drop(sdk);

    assert!(path.exists());
    let reloaded = build_mock_chain_sdk(&cfg).unwrap();
    assert_eq!(reloaded.get_participant(4).await.unwrap().participant_id, 4);
    assert_eq!(reloaded.snapshot().await.current_time, 1000);
}

#[tokio::test]
async fn concurrent_reads_and_updates_do_not_deadlock() {
    let handle = MockStateHandle::new(MockState::default());
    let sdk = build_mock_chain_sdk_with_state_handle(&config(None), handle.clone()).unwrap();
    seed(&handle).await;

    let writer = async {
        for _ in 0..50 {
            handle.update(|state| state.current_time += 1).await;
        }
    };
    let reader = async {
        for _ in 0..50 {
            let _ = sdk.get_participant(4).await;
        }
    };
    tokio::join!(writer, reader);
    assert_eq!(handle.snapshot().await.current_time, 1050);
}
