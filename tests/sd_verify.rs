//! Selective-disclosure verification (V1–V7): holder/VC signatures, disclosure hashes,
//! anchor lifecycle — using `did-sdk` v0.2 VP types and `VP-SD-v1`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use chain_sdk::config::{
    ChainSdkConfig, ChainSdkMode, DidConfig, KeyVaultConfig, KeyVaultKeys, ParticipantConfig,
    SdkRole,
};
use chain_sdk::sd::{SdDisclosure, SdExpected, SdIssueRequest, vp_signing_input};
use chain_sdk::{
    AnchorType, ChainSdk, ChainSdkError, MockChainSdk, MockState, SelectiveDisclosure,
    build_mock_chain_sdk_with_state,
};
use did_sdk::{Proof, VerifiableCredential, VerifiablePresentation};
use keyvault_core::{
    FileKeystore, KeyAlgorithm, KeyId, KeyNamespace, KeyVault, MemoryKeystore, PublicKey,
};
use secrecy::SecretString;

const CHALLENGE: &str = "challenge-1";
const DOMAIN: &str = "verifier.example.com";

fn temp_vc_path() -> std::path::PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    std::env::temp_dir().join(format!("chain-sdk-vc-{}-{seq}", std::process::id()))
}

fn fixture_state() -> MockState {
    MockState::from_json_str(include_str!("fixtures/mock_state.json")).unwrap()
}

async fn issuer_sdk() -> (tempfile::TempDir, Arc<MockChainSdk>) {
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

fn request(subject: &str, valid_until: u64) -> SdIssueRequest {
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
        valid_until,
        expiration_date: Some("2036-01-01T00:00:00Z".into()),
    }
}

struct Holder {
    vault: MemoryKeystore,
    key: KeyId,
    did: String,
}

async fn holder() -> Holder {
    let vault = MemoryKeystore::new();
    let key = vault
        .generate(KeyNamespace::ClientDid, KeyAlgorithm::Ed25519)
        .await
        .unwrap();
    let public = match vault.public_key(&key).await.unwrap() {
        PublicKey::Ed25519(bytes) => bytes,
        PublicKey::Sr25519(_) => panic!("holder key must be ed25519"),
    };
    let did = keyvault_core::crypto::encode_did_key(&public);
    Holder { vault, key, did }
}

async fn present(
    holder: &Holder,
    credential: &VerifiableCredential,
    disclosures: Vec<SdDisclosure>,
    challenge: &str,
) -> VerifiablePresentation {
    let input = vp_signing_input(
        &holder.did,
        challenge,
        DOMAIN,
        std::slice::from_ref(credential),
        &disclosures,
    )
    .unwrap();
    let signature = holder
        .vault
        .sign_checked(
            &holder.key,
            &input,
            KeyNamespace::ClientDid,
            KeyAlgorithm::Ed25519,
        )
        .await
        .unwrap();
    VerifiablePresentation {
        context: vec!["https://www.w3.org/ns/credentials/v2".into()],
        types: vec!["VerifiablePresentation".into()],
        holder: holder.did.clone(),
        verifiable_credential: vec![credential.clone()],
        challenge: challenge.into(),
        domain: DOMAIN.into(),
        disclosures: Some(disclosures),
        proof: Some(Proof {
            type_: "Ed25519Signature2020".into(),
            created: 0,
            verification_method: format!("{}#key-1", holder.did),
            jws: bs58::encode(signature.0).into_string(),
        }),
    }
}

fn expected() -> SdExpected {
    SdExpected {
        challenge: CHALLENGE.into(),
        domain: DOMAIN.into(),
    }
}

#[tokio::test]
async fn verify_subset_disclosure() {
    let (_dir, sdk) = issuer_sdk().await;
    let holder = holder().await;
    let issued = sdk
        .issue_sd_credential(request(&holder.did, 0))
        .await
        .unwrap();

    let selected: Vec<SdDisclosure> = issued
        .disclosures
        .iter()
        .filter(|disclosure| disclosure.name == "name" || disclosure.name == "gender")
        .cloned()
        .collect();
    let presentation = present(&holder, &issued.credential, selected, CHALLENGE).await;

    let report = sdk
        .verify_sd_presentation(&presentation, &expected())
        .await
        .unwrap();
    assert!(report.verified);
    assert_eq!(report.anchor_id, issued.anchor_id);
    assert_eq!(report.disclosed.len(), 2);
    assert_eq!(report.disclosed["name"], serde_json::json!("张三"));
    assert_eq!(report.disclosed["gender"], serde_json::json!("男"));
    assert_eq!(report.undisclosed_count, 4);
    assert!(!report.disclosed.contains_key("birthday"));
}

#[tokio::test]
async fn wrong_challenge_is_rejected() {
    let (_dir, sdk) = issuer_sdk().await;
    let holder = holder().await;
    let issued = sdk
        .issue_sd_credential(request(&holder.did, 0))
        .await
        .unwrap();
    let presentation = present(&holder, &issued.credential, vec![], "another-challenge").await;

    let error = sdk
        .verify_sd_presentation(&presentation, &expected())
        .await
        .unwrap_err();
    assert!(matches!(error, ChainSdkError::Unauthorized(_)));
}

#[tokio::test]
async fn tampered_disclosure_is_rejected() {
    let (_dir, sdk) = issuer_sdk().await;
    let holder = holder().await;
    let issued = sdk
        .issue_sd_credential(request(&holder.did, 0))
        .await
        .unwrap();

    let mut disclosures = issued.disclosures.clone();
    disclosures[0].value = serde_json::json!("李四");
    let presentation = present(&holder, &issued.credential, disclosures, CHALLENGE).await;

    let error = sdk
        .verify_sd_presentation(&presentation, &expected())
        .await
        .unwrap_err();
    assert!(matches!(error, ChainSdkError::DisclosureHashMismatch));
}

#[tokio::test]
async fn duplicate_disclosure_is_rejected() {
    let (_dir, sdk) = issuer_sdk().await;
    let holder = holder().await;
    let issued = sdk
        .issue_sd_credential(request(&holder.did, 0))
        .await
        .unwrap();

    let duplicate = issued.disclosures[0].clone();
    let presentation = present(
        &holder,
        &issued.credential,
        vec![duplicate.clone(), duplicate],
        CHALLENGE,
    )
    .await;

    let error = sdk
        .verify_sd_presentation(&presentation, &expected())
        .await
        .unwrap_err();
    assert!(matches!(error, ChainSdkError::DisclosureDuplicate(_)));
}

#[tokio::test]
async fn tampered_credential_is_rejected() {
    let (_dir, sdk) = issuer_sdk().await;
    let holder = holder().await;
    let issued = sdk
        .issue_sd_credential(request(&holder.did, 0))
        .await
        .unwrap();

    let mut credential = issued.credential.clone();
    credential.credential_subject["idCardNumber"] = serde_json::json!("000000000000000000");
    let presentation = present(&holder, &credential, vec![], CHALLENGE).await;

    let error = sdk
        .verify_sd_presentation(&presentation, &expected())
        .await
        .unwrap_err();
    assert!(matches!(error, ChainSdkError::InvalidArgument(_)));
}

#[tokio::test]
async fn revoked_anchor_is_rejected() {
    let (_dir, sdk) = issuer_sdk().await;
    let holder = holder().await;
    let issued = sdk
        .issue_sd_credential(request(&holder.did, 0))
        .await
        .unwrap();
    sdk.revoke_anchor(&issued.anchor_id).await.unwrap();
    let presentation = present(&holder, &issued.credential, vec![], CHALLENGE).await;

    let error = sdk
        .verify_sd_presentation(&presentation, &expected())
        .await
        .unwrap_err();
    assert!(matches!(error, ChainSdkError::AnchorRevoked(_)));
}

#[tokio::test]
async fn expired_anchor_is_rejected() {
    let (_dir, sdk) = issuer_sdk().await;
    let holder = holder().await;
    let issued = sdk
        .issue_sd_credential(request(&holder.did, 1))
        .await
        .unwrap();
    let presentation = present(&holder, &issued.credential, vec![], CHALLENGE).await;

    let error = sdk
        .verify_sd_presentation(&presentation, &expected())
        .await
        .unwrap_err();
    assert!(matches!(error, ChainSdkError::AnchorExpired(_)));
}
