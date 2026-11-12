//! S10 / S13: FileKeystore integration and `KeyVaultError` mapping.
//!
//! - Issuer builds with `file-keystore` and signs through the participant-did key;
//! - missing keys map to `SignerUnavailable` (via `map_vault_err`);
//! - wrong keystore password maps to `SignerUnavailable`, not a panic.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use chain_sdk::config::{
    ChainSdkConfig, ChainSdkMode, DidConfig, KeyVaultConfig, KeyVaultKeys, ParticipantConfig,
    SdkRole,
};
use chain_sdk::{AssertionPurpose, ChainSdk, ChainSdkError, build_mock_chain_sdk};
use keyvault_core::{FileKeystore, KeyAlgorithm, KeyNamespace, KeyVault};
use secrecy::SecretString;

const SELF_DID: &str = "did:web:mail.example.com:issuer";

fn temp_vc_path() -> std::path::PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    std::env::temp_dir().join(format!("chain-sdk-vc-{}-{seq}", std::process::id()))
}

fn config(keys: KeyVaultKeys, scheme: &str, path: std::path::PathBuf) -> ChainSdkConfig {
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
        key_vault: KeyVaultConfig {
            scheme: scheme.into(),
            path,
            password_env: "CHAIN_SDK_TEST_UNSET_PASSWORD".into(),
            password_file: temp_vc_path().with_extension("password"),
            allow_auto_generate_password: false,
            keys,
        },
        ..Default::default()
    }
}

#[tokio::test]
async fn file_keystore_issuer_signs() {
    let dir = tempfile::tempdir().unwrap();
    let keys_dir = dir.path().join("keys");
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
    drop(vault);

    let mut cfg = config(
        KeyVaultKeys {
            operator: chain_key.0,
            participant_did: did_key.0,
        },
        "file-keystore",
        keys_dir,
    );
    cfg.resolved_password = Some(password);

    let sdk = build_mock_chain_sdk(&cfg).unwrap();
    let signature = sdk
        .sign_participant_assertion(b"payload", AssertionPurpose::VcIssuance)
        .await
        .unwrap();
    assert_eq!(signature.0.len(), 64);
}

#[tokio::test]
async fn missing_keys_map_to_signer_unavailable() {
    let cfg = config(
        KeyVaultKeys {
            operator: "chain-account/missing".into(),
            participant_did: "participant-did/missing".into(),
        },
        "memory-keystore",
        temp_vc_path(),
    );
    let sdk = build_mock_chain_sdk(&cfg).unwrap();

    let error = sdk
        .sign_participant_assertion(b"payload", AssertionPurpose::VcIssuance)
        .await
        .unwrap_err();
    match error {
        ChainSdkError::SignerUnavailable(message) => {
            assert!(message.contains("key not found"), "{message}");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test]
async fn wrong_password_maps_to_signer_unavailable() {
    let dir = tempfile::tempdir().unwrap();
    let keys_dir = dir.path().join("keys");
    let vault = FileKeystore::new(
        keys_dir.clone(),
        SecretString::from("correct-password".to_owned()),
    )
    .unwrap();
    let chain_key = vault
        .generate(KeyNamespace::ChainAccount, KeyAlgorithm::Sr25519)
        .await
        .unwrap();
    let did_key = vault
        .generate(KeyNamespace::ParticipantDid, KeyAlgorithm::Ed25519)
        .await
        .unwrap();
    drop(vault);

    let mut cfg = config(
        KeyVaultKeys {
            operator: chain_key.0,
            participant_did: did_key.0,
        },
        "file-keystore",
        keys_dir,
    );
    cfg.resolved_password = Some(SecretString::from("wrong-password".to_owned()));

    let sdk = build_mock_chain_sdk(&cfg).unwrap();
    let error = sdk
        .sign_participant_assertion(b"payload", AssertionPurpose::VcIssuance)
        .await
        .unwrap_err();
    assert!(matches!(error, ChainSdkError::SignerUnavailable(_)));
}
