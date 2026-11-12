//! Selective-disclosure issuance (I1–I6): salted hashes, `_sd`, disclosures, unchanged anchor.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use base64::Engine as _;
use chain_sdk::config::{
    ChainSdkConfig, ChainSdkMode, DidConfig, KeyVaultConfig, KeyVaultKeys, ParticipantConfig,
    SdkRole,
};
use chain_sdk::sd::SdIssueRequest;
use chain_sdk::{
    AnchorType, ChainSdk, MockChainSdk, MockState, SdDisclosure, SelectiveDisclosure,
    build_mock_chain_sdk_with_state,
};
use keyvault_core::{FileKeystore, KeyAlgorithm, KeyNamespace, KeyVault, PublicKey};
use secrecy::SecretString;

fn temp_vc_path() -> std::path::PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    std::env::temp_dir().join(format!("chain-sdk-vc-{}-{seq}", std::process::id()))
}

fn fixture_state() -> MockState {
    MockState::from_json_str(include_str!("fixtures/mock_state.json")).unwrap()
}

pub(crate) async fn issuer_sdk() -> (tempfile::TempDir, Arc<MockChainSdk>) {
    let dir = tempfile::tempdir().unwrap();
    let password = SecretString::from("test-password".to_owned());
    let keys_dir = dir.path().join("keys");
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
    let issuer_did = keyvault_core::crypto::encode_did_key(&did_public);

    let config = ChainSdkConfig {
        mode: ChainSdkMode::Mock,
        role: SdkRole::Issuer,
        participant_id: 4,
        participant: ParticipantConfig {
            controller_did: issuer_did,
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
    let sdk = build_mock_chain_sdk_with_state(&config, fixture_state()).unwrap();
    (dir, sdk)
}

pub(crate) fn id_card_request(subject: &str) -> SdIssueRequest {
    let mandatory = serde_json::json!({ "idCardNumber": "110101199001011234" });
    SdIssueRequest {
        credential_type: "IDCardVC".into(),
        subject_did: subject.into(),
        mandatory_claims: mandatory.as_object().cloned().unwrap(),
        disclosable_claims: vec![
            ("name".into(), serde_json::json!("张三")),
            ("gender".into(), serde_json::json!("男")),
            ("birthday".into(), serde_json::json!("1990-01-01")),
            ("blood_type".into(), serde_json::json!("A")),
            ("ethnicity".into(), serde_json::json!("汉族")),
            ("birth_place".into(), serde_json::json!("北京")),
        ],
        anchor_type: AnchorType::IssuedVc,
        valid_until: 0,
        expiration_date: Some("2036-01-01T00:00:00Z".into()),
    }
}

#[tokio::test]
async fn issue_builds_sd_array_and_disclosures() {
    let (_dir, sdk) = issuer_sdk().await;
    let holder = keyvault_core::crypto::encode_did_key(&[9u8; 32]);
    let issued = sdk
        .issue_sd_credential(id_card_request(&holder))
        .await
        .unwrap();

    // I1/I2：_sd / _sd_alg 正确。
    let subject = &issued.credential.credential_subject;
    assert_eq!(subject["_sd"].as_array().unwrap().len(), 6);
    assert_eq!(subject["_sd_alg"], "sha-256");
    assert_eq!(
        subject["idCardNumber"],
        serde_json::json!("110101199001011234")
    );
    assert_eq!(subject["id"], serde_json::json!(holder));

    // I4：Disclosure 形状与 16 字节盐。
    assert_eq!(issued.disclosures.len(), 6);
    for disclosure in &issued.disclosures {
        let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(&disclosure.salt)
            .unwrap();
        assert_eq!(raw.len(), 16);
    }
    let salts: std::collections::HashSet<_> = issued
        .disclosures
        .iter()
        .map(|disclosure| disclosure.salt.clone())
        .collect();
    assert_eq!(salts.len(), 6);
    let digests: std::collections::HashSet<_> = subject["_sd"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap().to_string())
        .collect();
    assert_eq!(digests.len(), 6);

    // 未披露属性原文不进入 VC。
    let text = serde_json::to_string(&issued.credential).unwrap();
    for secret in ["张三", "1990-01-01", "汉族", "北京"] {
        assert!(!text.contains(secret), "VC must not contain {secret}");
    }

    // I5：锚定哈希覆盖 VC（含 _sd），且与链上记录一致。
    assert!(issued.payload_hash.starts_with("blake2b:"));
    let anchor = sdk.get_anchor(&issued.anchor_id).await.unwrap();
    assert_eq!(anchor.payload_hash, issued.payload_hash);
    assert_eq!(
        issued.credential.anchor_id.as_deref(),
        Some(issued.anchor_id.as_str())
    );
    assert!(!issued.credential.proof.as_ref().unwrap().jws.is_empty());
}

#[tokio::test]
async fn salts_are_independent_across_credentials() {
    let (_dir, sdk) = issuer_sdk().await;
    let holder = keyvault_core::crypto::encode_did_key(&[8u8; 32]);
    let first = sdk
        .issue_sd_credential(id_card_request(&holder))
        .await
        .unwrap();
    let second = sdk
        .issue_sd_credential(id_card_request(&holder))
        .await
        .unwrap();

    // P1：同一主体、同一属性，不同 VC 的盐/哈希不同。
    let names: Vec<&str> = first
        .disclosures
        .iter()
        .map(|disclosure| disclosure.name.as_str())
        .collect();
    for name in names {
        let left = first
            .disclosures
            .iter()
            .find(|disclosure| disclosure.name == name)
            .unwrap();
        let right = second
            .disclosures
            .iter()
            .find(|disclosure| disclosure.name == name)
            .unwrap();
        assert_ne!(left.salt, right.salt);
        assert_ne!(
            chain_sdk::sd::disclosure_digest(left).unwrap(),
            chain_sdk::sd::disclosure_digest(right).unwrap()
        );
    }
    assert_eq!(
        issued_disclosures_present(&first.disclosures),
        issued_disclosures_present(&second.disclosures)
    );
}

fn issued_disclosures_present(disclosures: &[SdDisclosure]) -> bool {
    !disclosures.is_empty()
}
