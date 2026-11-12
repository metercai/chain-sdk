//! S17：按角色装配（did-resolution 两角色均有；KeyVault / 断言签名仅 Issuer）。
//!
//! 行为断言（集成测试）+ 内部装配断言（`src/config.rs` 单元测试）共同覆盖。

#![allow(clippy::unwrap_used, clippy::expect_used)]

use chain_sdk::config::{
    ChainSdkConfig, ChainSdkMode, DidConfig, KeyVaultConfig, ParticipantConfig, SdkRole,
};
use chain_sdk::{AssertionPurpose, ChainSdkError, build_chain_sdk};

fn temp_vc_path() -> std::path::PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    std::env::temp_dir().join(format!("chain-sdk-vc-{}-{seq}", std::process::id()))
}

fn config(role: SdkRole) -> ChainSdkConfig {
    ChainSdkConfig {
        mode: ChainSdkMode::Mock,
        role,
        participant: ParticipantConfig {
            controller_did: "did:web:participant.example.com".into(),
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

#[tokio::test]
async fn verifier_resolves_did_key_without_vault() {
    let sdk = build_chain_sdk(&config(SdkRole::Verifier)).unwrap();
    let public = [7u8; 32];
    let did = keyvault_core::crypto::encode_did_key(&public);
    let resolved = sdk.resolve_did_key(&did).unwrap();
    assert_eq!(resolved.0, public);
}

#[tokio::test]
async fn verifier_cannot_sign_or_write() {
    let sdk = build_chain_sdk(&config(SdkRole::Verifier)).unwrap();
    assert!(matches!(
        sdk.sign_participant_assertion(b"payload", AssertionPurpose::VcIssuance)
            .await,
        Err(ChainSdkError::SignerUnavailable(_))
    ));
    assert!(matches!(
        sdk.create_anchor(chain_sdk::AnchorType::EmailBinding, "h", 0)
            .await,
        Err(ChainSdkError::Unauthorized(_))
    ));
}

#[tokio::test]
async fn issuer_backend_is_readable() {
    let sdk = build_chain_sdk(&config(SdkRole::Issuer)).unwrap();
    // Mock 后端可用（空状态）。
    let error = sdk.get_anchor("anchor:missing").await.unwrap_err();
    assert!(matches!(error, ChainSdkError::AnchorNotFound(_)));
    assert_eq!(sdk.health_check().await.unwrap().mode, "mock");
}

#[tokio::test]
async fn did_web_resolution_rejects_invalid_did() {
    let sdk = build_chain_sdk(&config(SdkRole::Verifier)).unwrap();
    let error = sdk.resolve_did_web("did:web:").await.unwrap_err();
    assert!(matches!(error, ChainSdkError::InvalidArgument(_)));
}

#[test]
fn build_requires_participant_did() {
    let mut config = config(SdkRole::Verifier);
    config.participant.controller_did.clear();
    let error = build_chain_sdk(&config).err().expect("expected error");
    assert!(matches!(error, ChainSdkError::ConfigError(_)));
}
