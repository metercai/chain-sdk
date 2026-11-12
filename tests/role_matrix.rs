//! S2 / S9：3 后端 × 2 角色矩阵。
//!
//! 断言：
//! - Verifier：所有写方法 `Unauthorized`；断言签名 `SignerUnavailable`；
//! - Issuer：Mock / Centralized 可写；Chain 写操作返回 `Unsupported`（**不是**
//!   `Unauthorized`，因为角色门禁已通过）；
//! - Chain 无节点时 `health_check` 回报不健康而不是 panic。

#![allow(clippy::unwrap_used, clippy::expect_used)]

use chain_sdk::config::{
    ChainSdkConfig, ChainSdkMode, DidConfig, KeyVaultConfig, KeyVaultKeys, ParticipantConfig,
    SdkRole, build_chain_sdk, build_mock_chain_sdk_with_state,
};
use chain_sdk::{AnchorType, AssertionPurpose, ChainSdk, ChainSdkError, MockState};
use keyvault_core::{FileKeystore, KeyAlgorithm, KeyNamespace, KeyVault};
use secrecy::SecretString;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn temp_vc_path() -> std::path::PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    std::env::temp_dir().join(format!("chain-sdk-vc-{}-{seq}", std::process::id()))
}

fn config(mode: ChainSdkMode, role: SdkRole, endpoint: &str) -> ChainSdkConfig {
    ChainSdkConfig {
        mode,
        role,
        participant_id: 4,
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
        centralized_endpoint: endpoint.to_string(),
        chain_endpoint: "ws://127.0.0.1:1".into(),
        cache_ttl_secs: 0,
        ..Default::default()
    }
}

fn fixture_state() -> MockState {
    MockState::from_json_str(include_str!("fixtures/mock_state.json")).unwrap()
}

#[tokio::test]
async fn verifier_is_read_only_in_all_backends() {
    let server = MockServer::start().await;
    for (mode, endpoint) in [
        (ChainSdkMode::Mock, server.uri()),
        (ChainSdkMode::Centralized, server.uri()),
        (ChainSdkMode::Chain, server.uri()),
    ] {
        let sdk = build_chain_sdk(&config(mode, SdkRole::Verifier, &endpoint)).unwrap();

        assert!(
            matches!(
                sdk.create_anchor(AnchorType::EmailBinding, "h", 0).await,
                Err(ChainSdkError::Unauthorized(_))
            ),
            "{mode:?}: verifier create_anchor must be Unauthorized"
        );
        assert!(matches!(
            sdk.update_authorized_domains(4, vec![]).await,
            Err(ChainSdkError::Unauthorized(_))
        ));
        assert!(matches!(
            sdk.sign_participant_assertion(b"payload", AssertionPurpose::VcIssuance)
                .await,
            Err(ChainSdkError::SignerUnavailable(_))
        ));
    }
}

#[tokio::test]
async fn issuer_matrix_mock_centralized_chain() {
    // Mock：可写。
    let mock = build_mock_chain_sdk_with_state(
        &config(ChainSdkMode::Mock, SdkRole::Issuer, "http://127.0.0.1:9"),
        fixture_state(),
    )
    .unwrap();
    let anchor = mock
        .create_anchor(AnchorType::EmailBinding, "blake2b:aa", 0)
        .await
        .unwrap();
    assert!(anchor.starts_with("anchor:"));

    // Centralized：可写（wiremock）。
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/anchors"))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(json!({ "anchor_id": "anchor:http" })),
        )
        .mount(&server)
        .await;
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
    drop(vault);

    let mut issuer_config = config(ChainSdkMode::Centralized, SdkRole::Issuer, &server.uri());
    issuer_config.key_vault = KeyVaultConfig {
        scheme: "file-keystore".into(),
        path: keys_dir,
        password_env: "CHAIN_SDK_TEST_UNSET_PASSWORD".into(),
        password_file: dir.path().join("keystore.password"),
        allow_auto_generate_password: false,
        keys: KeyVaultKeys {
            operator: chain_key.0,
            participant_did: did_key.0,
        },
    };
    issuer_config.resolved_password = Some(password);

    let centralized = build_chain_sdk(&issuer_config).unwrap();
    let anchor = centralized
        .create_anchor(AnchorType::EmailBinding, "blake2b:bb", 0)
        .await
        .unwrap();
    assert_eq!(anchor, "anchor:http");

    // Chain：角色门禁通过，但 POC 未实现写链 → Unsupported。
    let chain =
        build_chain_sdk(&config(ChainSdkMode::Chain, SdkRole::Issuer, &server.uri())).unwrap();
    let error = chain
        .create_anchor(AnchorType::EmailBinding, "blake2b:cc", 0)
        .await
        .unwrap_err();
    assert!(
        matches!(error, ChainSdkError::Unsupported(_)),
        "chain issuer write must be Unsupported, got {error:?}"
    );
}

#[tokio::test]
async fn chain_unreachable_health_is_unhealthy() {
    let sdk = build_chain_sdk(&config(
        ChainSdkMode::Chain,
        SdkRole::Verifier,
        "http://127.0.0.1:9",
    ))
    .unwrap();
    let health = sdk.health_check().await.unwrap();
    assert!(!health.healthy);
    assert_eq!(health.mode, "chain");
    assert!(health.detail.is_some());

    let error = sdk.get_anchor("a1").await.unwrap_err();
    assert!(matches!(error, ChainSdkError::Unsupported(_)));
}

#[tokio::test]
async fn chain_did_key_resolution_works_offline() {
    let sdk = build_chain_sdk(&config(
        ChainSdkMode::Chain,
        SdkRole::Verifier,
        "http://127.0.0.1:9",
    ))
    .unwrap();
    let public = [3u8; 32];
    let did = keyvault_core::crypto::encode_did_key(&public);
    assert_eq!(sdk.resolve_did_key(&did).unwrap().0, public);
}
