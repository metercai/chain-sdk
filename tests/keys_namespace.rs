//! S14：两类密钥 namespace 前缀强制校验。
//!
//! 覆盖：
//! - `sign_extrinsic` 拒绝 `participant-did/` 与未知前缀；
//! - `sign_participant_assertion_inner` 拒绝 `chain-account/`；
//! - `build_chain_sdk` 在构造期拒绝错误前缀；
//! - 用 FileKeystore 生成真实密钥后，Issuer 可通过 trait 完成断言签名（S10 预演）。

#![allow(clippy::unwrap_used, clippy::expect_used)]

use chain_sdk::config::{
    ChainSdkConfig, ChainSdkMode, DidConfig, KeyVaultConfig, KeyVaultKeys, ParticipantConfig,
    SdkRole,
};
use chain_sdk::keys::{
    CHAIN_ACCOUNT_PREFIX, PARTICIPANT_DID_PREFIX, sign_extrinsic, sign_participant_assertion_inner,
};
use chain_sdk::{AssertionPurpose, ChainSdkError, build_chain_sdk};
use keyvault_core::{FileKeystore, KeyAlgorithm, KeyId, KeyNamespace, KeyVault, PublicKey};
use secrecy::SecretString;

const SELF_DID: &str = "did:web:mail.example.com:issuer";

fn temp_vc_path() -> std::path::PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    std::env::temp_dir().join(format!("chain-sdk-vc-{}-{seq}", std::process::id()))
}

fn issuer_config(scheme: &str, operator: String, participant_did: String) -> ChainSdkConfig {
    ChainSdkConfig {
        mode: ChainSdkMode::Mock,
        role: SdkRole::Issuer,
        participant: ParticipantConfig {
            controller_did: SELF_DID.into(),
        },
        did: DidConfig {
            vc_storage_path: temp_vc_path(),
        },
        key_vault: KeyVaultConfig {
            scheme: scheme.into(),
            keys: KeyVaultKeys {
                operator,
                participant_did,
            },
            ..Default::default()
        },
        ..Default::default()
    }
}

#[tokio::test]
async fn sign_extrinsic_rejects_participant_did_namespace() {
    let vault = keyvault_core::MemoryKeystore::new();
    let bad = KeyId(format!("{PARTICIPANT_DID_PREFIX}foo"));
    let error = sign_extrinsic(&vault, &bad, b"payload").await.unwrap_err();
    assert!(matches!(error, ChainSdkError::KeyNamespaceViolation(_)));
}

#[tokio::test]
async fn sign_extrinsic_rejects_unknown_namespace() {
    let vault = keyvault_core::MemoryKeystore::new();
    let bad = KeyId("other/foo".into());
    let error = sign_extrinsic(&vault, &bad, b"payload").await.unwrap_err();
    assert!(matches!(error, ChainSdkError::KeyNamespaceViolation(_)));
}

#[tokio::test]
async fn sign_participant_assertion_rejects_chain_account_namespace() {
    let vault = keyvault_core::MemoryKeystore::new();
    let bad = KeyId(format!("{CHAIN_ACCOUNT_PREFIX}operator-1"));
    let error =
        sign_participant_assertion_inner(&vault, &bad, b"payload", AssertionPurpose::VcIssuance)
            .await
            .unwrap_err();
    assert!(matches!(error, ChainSdkError::KeyNamespaceViolation(_)));
}

#[test]
fn build_chain_sdk_rejects_wrong_operator_prefix() {
    let config = issuer_config(
        "memory-keystore",
        format!("{PARTICIPANT_DID_PREFIX}wrong"),
        format!("{PARTICIPANT_DID_PREFIX}ok"),
    );
    let error = build_chain_sdk(&config).err().expect("expected error");
    assert!(matches!(error, ChainSdkError::KeyNamespaceViolation(_)));
}

#[test]
fn build_chain_sdk_rejects_wrong_participant_did_prefix() {
    let config = issuer_config(
        "memory-keystore",
        format!("{CHAIN_ACCOUNT_PREFIX}operator-1"),
        format!("{CHAIN_ACCOUNT_PREFIX}wrong"),
    );
    let error = build_chain_sdk(&config).err().expect("expected error");
    assert!(matches!(error, ChainSdkError::KeyNamespaceViolation(_)));
}

#[tokio::test]
async fn verifier_has_no_signer() {
    let config = ChainSdkConfig {
        role: SdkRole::Verifier,
        mode: ChainSdkMode::Mock,
        participant: ParticipantConfig {
            controller_did: "did:web:verifier.example.com".into(),
        },
        did: DidConfig {
            vc_storage_path: temp_vc_path(),
        },
        ..Default::default()
    };
    let sdk = build_chain_sdk(&config).unwrap();
    let error = sdk
        .sign_participant_assertion(b"payload", AssertionPurpose::VcIssuance)
        .await
        .unwrap_err();
    assert!(matches!(error, ChainSdkError::SignerUnavailable(_)));
}

#[tokio::test]
async fn issuer_signs_with_file_keystore() {
    let dir = tempfile::tempdir().unwrap();
    let keys_dir = dir.path().join("participant-keys");
    let password = SecretString::from("test-password".to_owned());

    let vault = FileKeystore::new(keys_dir.clone(), password.clone()).unwrap();
    let chain_key = vault
        .generate(KeyNamespace::ChainAccount, KeyAlgorithm::Sr25519)
        .await
        .unwrap();
    let did_key = vault
        .generate(KeyNamespace::ParticipantDid, KeyAlgorithm::Ed25519)
        .await
        .unwrap();
    let did_public = match vault.public_key(&did_key).await.unwrap() {
        PublicKey::Ed25519(bytes) => bytes,
        PublicKey::Sr25519(_) => panic!("participant-did key must be ed25519"),
    };
    drop(vault);

    let config = ChainSdkConfig {
        mode: ChainSdkMode::Mock,
        role: SdkRole::Issuer,
        participant: ParticipantConfig {
            controller_did: SELF_DID.into(),
        },
        did: DidConfig {
            vc_storage_path: temp_vc_path(),
        },
        key_vault: KeyVaultConfig {
            scheme: "file-keystore".into(),
            path: keys_dir,
            password_env: "CHAIN_SDK_TEST_UNSET_PASSWORD".into(),
            password_file: dir.path().join("keystore.password"),
            allow_auto_generate_password: false,
            keys: KeyVaultKeys {
                operator: chain_key.0,
                participant_did: did_key.0,
            },
        },
        resolved_password: Some(password),
        ..Default::default()
    };

    let sdk = build_chain_sdk(&config).unwrap();
    let signature = sdk
        .sign_participant_assertion(b"payload", AssertionPurpose::VcIssuance)
        .await
        .unwrap();
    assert_eq!(signature.0.len(), 64);
    assert!(keyvault_core::crypto::ed25519::verify(
        &did_public,
        b"payload",
        &signature.0
    ));
}
