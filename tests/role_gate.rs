//! S9 / S19：角色门禁与自身管理写范围。
//!
//! 覆盖：
//! - Verifier 调任何写方法 → `Unauthorized`；断言签名 → `SignerUnavailable`；
//! - Issuer 修改他人 → `Unauthorized`；
//! - Issuer 更新自身域名 / controller_did（含双向索引迁移）；
//! - 锚定生命周期：创建、撤销、续期、永久锚定、未注册类型；
//! - 写操作不改变状态当且仅当被拒绝；写后持久化。

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use chain_sdk::config::{
    ChainSdkConfig, ChainSdkMode, DidConfig, KeyVaultConfig, MockConfig, ParticipantConfig, SdkRole,
};
use chain_sdk::{
    AnchorType, AssertionPurpose, ChainSdk, ChainSdkError, MockChainSdk, MockState,
    build_mock_chain_sdk, build_mock_chain_sdk_with_state,
};

const SELF_DID: &str = "did:web:mail.example.com:issuer";

fn temp_vc_path() -> std::path::PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    std::env::temp_dir().join(format!("chain-sdk-vc-{}-{seq}", std::process::id()))
}

fn issuer_config(participant_id: u32, state_path: Option<PathBuf>) -> ChainSdkConfig {
    ChainSdkConfig {
        mode: ChainSdkMode::Mock,
        role: SdkRole::Issuer,
        participant_id,
        participant: ParticipantConfig {
            controller_did: SELF_DID.into(),
        },
        did: DidConfig {
            vc_storage_path: temp_vc_path(),
        },
        key_vault: KeyVaultConfig {
            scheme: "memory-keystore".into(),
            ..Default::default()
        },
        mock: MockConfig { state_path },
        ..Default::default()
    }
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

fn issuer_with_fixture() -> std::sync::Arc<MockChainSdk> {
    build_mock_chain_sdk_with_state(&issuer_config(4, None), fixture_state()).unwrap()
}

#[tokio::test]
async fn verifier_cannot_write_any_backend() {
    let sdk = build_mock_chain_sdk(&verifier_config()).unwrap();

    assert!(matches!(
        sdk.create_anchor(AnchorType::EmailBinding, "h", 0).await,
        Err(ChainSdkError::Unauthorized(_))
    ));
    assert!(matches!(
        sdk.revoke_anchor("anchor:1").await,
        Err(ChainSdkError::Unauthorized(_))
    ));
    assert!(matches!(
        sdk.renew_anchor("anchor:1", 1).await,
        Err(ChainSdkError::Unauthorized(_))
    ));
    assert!(matches!(
        sdk.update_authorized_domains(4, vec!["a.com".into()]).await,
        Err(ChainSdkError::Unauthorized(_))
    ));
    assert!(matches!(
        sdk.update_controller_did(4, "did:web:evil.example.com")
            .await,
        Err(ChainSdkError::Unauthorized(_))
    ));
    assert!(matches!(
        sdk.sign_participant_assertion(b"payload", AssertionPurpose::VcIssuance)
            .await,
        Err(ChainSdkError::SignerUnavailable(_))
    ));
}

#[tokio::test]
async fn verifier_write_does_not_mutate_state() {
    let mut state = fixture_state();
    state.anchors.clear();
    let sdk = build_mock_chain_sdk_with_state(&verifier_config(), state).unwrap();
    let before = sdk.snapshot().await.anchors.len();

    let _ = sdk.create_anchor(AnchorType::EmailBinding, "h", 0).await;
    assert_eq!(sdk.snapshot().await.anchors.len(), before);
}

#[tokio::test]
async fn issuer_cannot_manage_other_participants() {
    let sdk = issuer_with_fixture();
    assert!(matches!(
        sdk.update_authorized_domains(999, vec![]).await,
        Err(ChainSdkError::Unauthorized(_))
    ));
    assert!(matches!(
        sdk.update_controller_did(999, "did:web:evil.example.com")
            .await,
        Err(ChainSdkError::Unauthorized(_))
    ));
}

#[tokio::test]
async fn issuer_updates_own_domains() {
    let sdk = issuer_with_fixture();
    sdk.update_authorized_domains(4, vec!["mail.example.com".into(), "example.com".into()])
        .await
        .unwrap();

    let participant = sdk.get_participant(4).await.unwrap();
    assert_eq!(
        participant.authorized_domains,
        vec!["mail.example.com", "example.com"]
    );
}

#[tokio::test]
async fn issuer_updates_own_controller_did_and_index() {
    let sdk = issuer_with_fixture();
    sdk.update_controller_did(4, "did:web:new.example.com")
        .await
        .unwrap();

    let moved = sdk
        .get_participant_by_did("did:web:new.example.com")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(moved.participant_id, 4);

    assert!(
        sdk.get_participant_by_did("did:web:mail.example.com:issuer")
            .await
            .unwrap()
            .is_none()
    );

    // 不能抢占其他参与方的 DID。
    let error = sdk
        .update_controller_did(4, "did:web:proxy.example.com")
        .await
        .unwrap_err();
    assert!(matches!(error, ChainSdkError::InvalidArgument(_)));
}

#[tokio::test]
async fn anchor_lifecycle() {
    let sdk = issuer_with_fixture();

    let anchor_id = sdk
        .create_anchor(AnchorType::EmailBinding, "blake2b:1234", 0)
        .await
        .unwrap();
    assert_eq!(anchor_id, "anchor:2");

    let anchor = sdk.get_anchor(&anchor_id).await.unwrap();
    assert_eq!(anchor.submitter_id, 4);
    assert_eq!(anchor.issuer_did, SELF_DID);
    assert!(anchor.delegated_by_did.is_none());
    assert_eq!(anchor.payload_hash, "blake2b:1234");
    assert!(!anchor.revoked);
    assert_eq!(anchor.created_at, 1000);
    assert_eq!(anchor.valid_until, 0);

    sdk.revoke_anchor(&anchor_id).await.unwrap();
    assert!(sdk.get_anchor(&anchor_id).await.unwrap().revoked);
    assert!(matches!(
        sdk.revoke_anchor(&anchor_id).await,
        Err(ChainSdkError::AnchorRevoked(_))
    ));
    assert!(matches!(
        sdk.renew_anchor(&anchor_id, 10).await,
        Err(ChainSdkError::AnchorRevoked(_))
    ));

    let finite = sdk
        .create_anchor(AnchorType::DomainControl, "blake2b:5678", 100)
        .await
        .unwrap();
    assert!(matches!(
        sdk.get_anchor(&finite).await.unwrap().valid_until,
        100
    ));
    sdk.renew_anchor(&finite, 200).await.unwrap();
    assert_eq!(sdk.get_anchor(&finite).await.unwrap().valid_until, 200);
    assert!(matches!(
        sdk.renew_anchor(&finite, 0).await,
        Err(ChainSdkError::InvalidArgument(_))
    ));
}

#[tokio::test]
async fn permanent_anchor_cannot_be_renewed() {
    let sdk = issuer_with_fixture();
    let error = sdk.renew_anchor("anchor:1", 100).await.unwrap_err();
    assert!(matches!(error, ChainSdkError::AnchorPermanent(_)));
}

#[tokio::test]
async fn unregistered_anchor_type_is_rejected() {
    let sdk = issuer_with_fixture();
    let error = sdk
        .create_anchor(AnchorType::Custom("NotRegistered".into()), "h", 0)
        .await
        .unwrap_err();
    assert!(matches!(error, ChainSdkError::AnchorTypeNotRegistered(_)));
}

#[tokio::test]
async fn inactive_participant_cannot_self_manage() {
    let mut state = fixture_state();
    state.participants.get_mut(&4).unwrap().active = false;
    let sdk = build_mock_chain_sdk_with_state(&issuer_config(4, None), state).unwrap();

    assert!(matches!(
        sdk.update_authorized_domains(4, vec![]).await,
        Err(ChainSdkError::ParticipantInactive(4))
    ));
}

#[tokio::test]
async fn writes_are_persisted() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mock_state.json");
    std::fs::write(&path, include_str!("fixtures/mock_state.json")).unwrap();

    let config = issuer_config(4, Some(path.clone()));
    let sdk = build_mock_chain_sdk(&config).unwrap();
    let anchor_id = sdk
        .create_anchor(AnchorType::IssuedVc, "blake2b:persist", 0)
        .await
        .unwrap();

    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(raw.contains(&anchor_id), "{raw}");

    let reloaded = build_mock_chain_sdk(&config).unwrap();
    let anchor = reloaded.get_anchor(&anchor_id).await.unwrap();
    assert_eq!(anchor.payload_hash, "blake2b:persist");
}
