//! CS-01: caller-provided `KeyVault` injection.
//!
//! The same vault instance must be used by the SDK and the host; a second
//! `FileKeystore` must not be opened, and `Verifier + vault` is rejected.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chain_sdk::config::{
    ChainSdkConfig, ChainSdkMode, DidConfig, KeyVaultConfig, KeyVaultKeys, ParticipantConfig,
    SdkRole,
};
use chain_sdk::keys::sign_extrinsic;
use chain_sdk::{
    AssertionPurpose, ChainSdk, ChainSdkError, build_chain_sdk_with_vault,
    build_mock_chain_sdk_with_vault,
};
use keyvault_core::{FileKeystore, KeyAlgorithm, KeyNamespace, KeyVault};
use secrecy::SecretString;

const SELF_DID: &str = "did:web:mail.example.com:issuer";

fn temp_vc_path() -> PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    std::env::temp_dir().join(format!("chain-sdk-vc-{}-{seq}", std::process::id()))
}

fn config(keys: KeyVaultKeys, keys_dir: PathBuf) -> ChainSdkConfig {
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
            scheme: "file-keystore".into(),
            path: keys_dir,
            password_env: "CHAIN_SDK_TEST_UNSET_PASSWORD".into(),
            password_file: temp_vc_path().with_extension("password"),
            allow_auto_generate_password: false,
            keys,
        },
        resolved_password: Some(SecretString::from("test-password".to_owned())),
        ..Default::default()
    }
}

fn tree(root: &Path) -> Vec<String> {
    fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, base, out);
            } else {
                let name = path
                    .strip_prefix(base)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .to_string();
                let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                out.push(format!("{name}:{size}"));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

#[tokio::test]
async fn injected_vault_is_reused_without_a_second_keystore() {
    let dir = tempfile::tempdir().unwrap();
    let keys_dir = dir.path().join("keys");
    let vault: Arc<dyn KeyVault> = Arc::new(
        FileKeystore::new(
            keys_dir.clone(),
            SecretString::from("test-password".to_owned()),
        )
        .unwrap(),
    );
    let chain_key = vault
        .generate(KeyNamespace::ChainAccount, KeyAlgorithm::Sr25519)
        .await
        .unwrap();
    let did_key = vault
        .generate(KeyNamespace::ParticipantDid, KeyAlgorithm::Ed25519)
        .await
        .unwrap();
    let cfg = config(
        KeyVaultKeys {
            operator: chain_key.0.clone(),
            participant_did: did_key.0.clone(),
        },
        keys_dir.clone(),
    );

    let before = tree(&keys_dir);
    let sdk = build_mock_chain_sdk_with_vault(&cfg, Arc::clone(&vault)).unwrap();

    let signature = sdk
        .sign_participant_assertion(b"payload", AssertionPurpose::VcIssuance)
        .await
        .unwrap();
    assert_eq!(signature.0.len(), 64);
    assert_eq!(
        sign_extrinsic(vault.as_ref(), &chain_key, b"call")
            .await
            .unwrap()
            .0
            .len(),
        64
    );

    // Cross-namespace use is still rejected.
    assert!(matches!(
        sign_extrinsic(vault.as_ref(), &did_key, b"call").await,
        Err(ChainSdkError::KeyNamespaceViolation(_))
    ));

    // Building the SDK and signing did not create another keystore.
    assert_eq!(before, tree(&keys_dir));
}

#[tokio::test]
async fn verifier_with_vault_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let keys_dir = dir.path().join("keys");
    let vault: Arc<dyn KeyVault> = Arc::new(
        FileKeystore::new(
            keys_dir.clone(),
            SecretString::from("test-password".to_owned()),
        )
        .unwrap(),
    );
    let mut cfg = config(
        KeyVaultKeys {
            operator: "chain-account/x".into(),
            participant_did: "participant-did/x".into(),
        },
        keys_dir,
    );
    cfg.role = SdkRole::Verifier;

    let error = build_chain_sdk_with_vault(&cfg, vault)
        .err()
        .expect("expected ConfigError");
    assert!(matches!(error, ChainSdkError::ConfigError(_)));
}

#[tokio::test]
async fn build_key_vault_returns_usable_memory_keystore() {
    let mut cfg = config(
        KeyVaultKeys {
            operator: "chain-account/x".into(),
            participant_did: "participant-did/x".into(),
        },
        temp_vc_path(),
    );
    cfg.key_vault.scheme = "memory-keystore".into();
    let vault = chain_sdk::build_key_vault(&cfg).unwrap();
    let key = vault
        .generate(KeyNamespace::ChainAccount, KeyAlgorithm::Sr25519)
        .await
        .unwrap();
    assert_eq!(
        sign_extrinsic(vault.as_ref(), &key, b"call")
            .await
            .unwrap()
            .0
            .len(),
        64
    );
}

#[tokio::test]
async fn build_key_vault_rejects_unknown_scheme() {
    let mut cfg = config(
        KeyVaultKeys {
            operator: "chain-account/x".into(),
            participant_did: "participant-did/x".into(),
        },
        temp_vc_path(),
    );
    cfg.key_vault.scheme = "bogus".into();
    let error = chain_sdk::build_key_vault(&cfg)
        .err()
        .expect("expected ConfigError");
    assert!(matches!(error, ChainSdkError::ConfigError(_)));
}
