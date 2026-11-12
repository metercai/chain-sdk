//! S15 / S16 / S20：参与方身份三段式绑定、委托锚定语义、`business_types` 不参与鉴权。
//!
//! 覆盖：
//! - `controller_did` ↔ `participant_id` ↔ `operator` 双向索引；
//! - 委托锚定：`issuer_did` = 信任根、`submitter_id` = 代理方、`delegated_by_did`；
//! - 修改 `business_types` 不改变鉴权结果。

#![allow(clippy::unwrap_used, clippy::expect_used)]

use chain_sdk::config::{
    ChainSdkConfig, ChainSdkMode, DidConfig, KeyVaultConfig, ParticipantConfig, SdkRole,
};
use chain_sdk::{AnchorRecord, ChainRole, ChainSdk, MockState, build_mock_chain_sdk_with_state};

fn temp_vc_path() -> std::path::PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    std::env::temp_dir().join(format!("chain-sdk-vc-{}-{seq}", std::process::id()))
}

fn issuer_config() -> ChainSdkConfig {
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
async fn participant_identity_is_hard_bound() {
    let sdk = build_mock_chain_sdk_with_state(&verifier_config(), fixture_state()).unwrap();

    let participant = sdk.get_participant(4).await.unwrap();
    assert_eq!(participant.participant_id, 4);
    assert_eq!(
        participant.controller_did,
        "did:web:mail.example.com:issuer"
    );
    assert_eq!(participant.operator, "5GrwvaEF...");
    assert_eq!(participant.chain_role, ChainRole::Issuer);

    let by_did = sdk
        .get_participant_by_did("did:web:mail.example.com:issuer")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(by_did.participant_id, 4);

    let state = sdk.snapshot().await;
    assert_eq!(
        state.account_participant.get("5GrwvaEF...").copied(),
        Some(4)
    );
    assert_eq!(
        state
            .did_participant
            .get("did:web:proxy.example.com")
            .copied(),
        Some(7)
    );
}

#[tokio::test]
async fn delegated_anchor_keeps_issuer_did_as_trust_root() {
    let sdk = build_mock_chain_sdk_with_state(&verifier_config(), fixture_state()).unwrap();
    let anchor = sdk.get_anchor("anchor:delegated-1").await.unwrap();

    // 语义签发者 = 信任链根，不是提交者。
    assert_eq!(anchor.issuer_did, "did:key:zA");
    // 提交者是代理方（治理审计）。
    assert_eq!(anchor.submitter_id, 7);
    // 代理方透明可见。
    assert_eq!(
        anchor.delegated_by_did.as_deref(),
        Some("did:web:proxy.example.com")
    );

    // 独立 fixture 与状态内记录一致。
    let fixture: AnchorRecord =
        serde_json::from_str(include_str!("fixtures/delegated_anchor.json")).unwrap();
    assert_eq!(fixture, anchor);
}

#[tokio::test]
async fn own_anchor_has_no_delegate() {
    let sdk = build_mock_chain_sdk_with_state(&issuer_config(), fixture_state()).unwrap();
    let anchor = sdk
        .create_anchor(chain_sdk::AnchorType::EmailBinding, "h", 0)
        .await
        .unwrap();
    let record = sdk.get_anchor(&anchor).await.unwrap();
    assert_eq!(record.issuer_did, "did:web:mail.example.com:issuer");
    assert_eq!(record.submitter_id, 4);
    assert!(record.delegated_by_did.is_none());
}

#[tokio::test]
async fn business_types_do_not_affect_authorization() {
    let mut state = fixture_state();
    // 去掉所有 business_types：鉴权不应变化。
    state
        .participants
        .get_mut(&4)
        .unwrap()
        .business_types
        .clear();
    let sdk = build_mock_chain_sdk_with_state(&issuer_config(), state).unwrap();

    sdk.update_authorized_domains(4, vec!["example.com".into()])
        .await
        .unwrap();
    sdk.update_controller_did(4, "did:web:renamed.example.com")
        .await
        .unwrap();

    // 反向：把 business_types 改成看起来“有权限”的类型，也不能越权修改他人。
    let mut state = fixture_state();
    state.participants.get_mut(&4).unwrap().business_types =
        vec![chain_sdk::BusinessType::ChainOwner];
    let sdk = build_mock_chain_sdk_with_state(&issuer_config(), state).unwrap();
    assert!(matches!(
        sdk.update_authorized_domains(7, vec![]).await,
        Err(chain_sdk::ChainSdkError::Unauthorized(_))
    ));
}

#[tokio::test]
async fn controller_did_rotation_preserves_operator_binding() {
    let sdk = build_mock_chain_sdk_with_state(&issuer_config(), fixture_state()).unwrap();
    sdk.update_controller_did(4, "did:web:rotated.example.com")
        .await
        .unwrap();

    let participant = sdk.get_participant(4).await.unwrap();
    assert_eq!(participant.controller_did, "did:web:rotated.example.com");
    assert_eq!(participant.operator, "5GrwvaEF...");

    let state = sdk.snapshot().await;
    assert_eq!(
        state.account_participant.get("5GrwvaEF...").copied(),
        Some(4)
    );
    assert!(
        !state
            .did_participant
            .contains_key("did:web:mail.example.com:issuer")
    );
}
