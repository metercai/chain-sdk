//! S7：Centralized 写请求签名（`x-chain-signer` / `x-chain-signature` / `x-chain-op`）。
//!
//! 覆盖：
//! - `op || "\n" || canonical_json(body)` 待签名字节可用 sr25519 公钥验证；
//! - 每个写端点 method + path 正确；
//! - `x-chain-*` 头存在且格式正确；
//! - Verifier 写操作在发出任何 HTTP 请求之前被拒绝（S9）。

#![allow(clippy::unwrap_used, clippy::expect_used)]

use chain_sdk::config::{
    ChainSdkConfig, ChainSdkMode, DidConfig, KeyVaultConfig, KeyVaultKeys, ParticipantConfig,
    SdkRole, build_centralized_chain_sdk,
};
use chain_sdk::{AnchorType, ChainSdk, ChainSdkError};
use keyvault_core::{FileKeystore, KeyAlgorithm, KeyNamespace, KeyVault, PublicKey};
use secrecy::SecretString;
use serde_json::Value;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn temp_vc_path() -> std::path::PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    std::env::temp_dir().join(format!("chain-sdk-vc-{}-{seq}", std::process::id()))
}

fn canonical(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<_> = map.iter().collect();
            entries.sort_by(|left, right| left.0.cmp(right.0));
            let fields: Vec<String> = entries
                .into_iter()
                .map(|(key, value)| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(key).unwrap(),
                        canonical(value)
                    )
                })
                .collect();
            format!("{{{}}}", fields.join(","))
        }
        Value::Array(items) => {
            let items: Vec<String> = items.iter().map(canonical).collect();
            format!("[{}]", items.join(","))
        }
        other => serde_json::to_string(other).unwrap(),
    }
}

struct IssuerFixture {
    _dir: tempfile::TempDir,
    config: ChainSdkConfig,
    chain_public: [u8; 32],
    signer_id: String,
}

async fn issuer_fixture(server: &MockServer, ttl_secs: u64) -> IssuerFixture {
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
    let chain_public = match vault.public_key(&chain_key).await.unwrap() {
        PublicKey::Sr25519(bytes) => bytes,
        PublicKey::Ed25519(_) => panic!("chain-account key must be sr25519"),
    };
    drop(vault);

    let config = ChainSdkConfig {
        mode: ChainSdkMode::Centralized,
        role: SdkRole::Issuer,
        participant_id: 4,
        participant: ParticipantConfig {
            controller_did: "did:web:mail.example.com:issuer".into(),
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
                operator: chain_key.0.clone(),
                participant_did: did_key.0,
            },
        },
        resolved_password: Some(password),
        centralized_endpoint: server.uri(),
        cache_ttl_secs: ttl_secs,
        ..Default::default()
    };

    IssuerFixture {
        _dir: dir,
        config,
        chain_public,
        signer_id: chain_key.0,
    }
}

fn verifier_config(server: &MockServer) -> ChainSdkConfig {
    ChainSdkConfig {
        mode: ChainSdkMode::Centralized,
        role: SdkRole::Verifier,
        participant: ParticipantConfig {
            controller_did: "did:web:verifier.example.com".into(),
        },
        did: DidConfig {
            vc_storage_path: temp_vc_path(),
        },
        centralized_endpoint: server.uri(),
        ..Default::default()
    }
}

#[tokio::test]
async fn create_anchor_signs_canonical_request() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/anchors"))
        .respond_with(
            ResponseTemplate::new(201)
                .set_body_json(serde_json::json!({ "anchor_id": "anchor:9" })),
        )
        .mount(&server)
        .await;

    let fixture = issuer_fixture(&server, 60).await;
    let sdk = build_centralized_chain_sdk(&fixture.config).unwrap();
    let anchor_id = sdk
        .create_anchor(AnchorType::EmailBinding, "blake2b:deadbeef", 100)
        .await
        .unwrap();
    assert_eq!(anchor_id, "anchor:9");

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];

    assert_eq!(
        request.headers.get("x-chain-op").unwrap().to_str().unwrap(),
        "create_anchor"
    );
    assert_eq!(
        request
            .headers
            .get("x-chain-signer")
            .unwrap()
            .to_str()
            .unwrap(),
        fixture.signer_id
    );

    let signature = hex::decode(
        request
            .headers
            .get("x-chain-signature")
            .unwrap()
            .to_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(signature.len(), 64);

    let body: Value = serde_json::from_slice(&request.body).unwrap();
    let canonical = canonical(&body);
    // 发出的 body 本身就是 canonical JSON。
    assert_eq!(request.body, canonical.as_bytes());

    let payload = format!("create_anchor\n{canonical}");
    assert!(
        keyvault_core::crypto::sr25519::verify(
            &fixture.chain_public,
            payload.as_bytes(),
            &signature
        ),
        "signature must verify over op || \\n || canonical body"
    );
}

#[tokio::test]
async fn revoke_and_renew_hit_expected_paths_and_invalidate_cache() {
    let server = MockServer::start().await;
    let anchor = serde_json::json!({
        "anchor_id": "a1",
        "anchor_type": { "kind": "EmailBinding" },
        "submitter_id": 4,
        "issuer_did": "did:web:mail.example.com:issuer",
        "delegated_by_did": null,
        "payload_hash": "blake2b:deadbeef",
        "revoked": false,
        "created_at": 1000,
        "valid_until": 100
    });
    Mock::given(method("GET"))
        .and(path("/anchors/a1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&anchor))
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/anchors/a1/revoke"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/anchors/a1/renew"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .expect(1)
        .mount(&server)
        .await;

    let fixture = issuer_fixture(&server, 60).await;
    let sdk = build_centralized_chain_sdk(&fixture.config).unwrap();

    sdk.get_anchor("a1").await.unwrap(); // GET 1（写缓存）
    sdk.revoke_anchor("a1").await.unwrap(); // POST + 失效
    sdk.get_anchor("a1").await.unwrap(); // GET 2
    sdk.renew_anchor("a1", 200).await.unwrap(); // POST

    server.verify().await;
}

#[tokio::test]
async fn participant_update_paths() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/participants/4/domains"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/participants/4/controller-did"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .expect(1)
        .mount(&server)
        .await;

    let fixture = issuer_fixture(&server, 60).await;
    let sdk = build_centralized_chain_sdk(&fixture.config).unwrap();
    sdk.update_authorized_domains(4, vec!["example.com".into()])
        .await
        .unwrap();
    sdk.update_controller_did(4, "did:web:new.example.com")
        .await
        .unwrap();

    server.verify().await;
}

#[tokio::test]
async fn issuer_cannot_update_others() {
    let server = MockServer::start().await;
    let fixture = issuer_fixture(&server, 60).await;
    let sdk = build_centralized_chain_sdk(&fixture.config).unwrap();

    assert!(matches!(
        sdk.update_authorized_domains(999, vec![]).await,
        Err(ChainSdkError::Unauthorized(_))
    ));
    assert!(matches!(
        sdk.update_controller_did(999, "did:web:evil.example.com")
            .await,
        Err(ChainSdkError::Unauthorized(_))
    ));
    assert!(
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty()
    );
}

#[tokio::test]
async fn verifier_write_is_rejected_before_http() {
    let server = MockServer::start().await;
    let sdk = build_centralized_chain_sdk(&verifier_config(&server)).unwrap();

    let error = sdk
        .create_anchor(AnchorType::EmailBinding, "h", 0)
        .await
        .unwrap_err();
    assert!(matches!(error, ChainSdkError::Unauthorized(_)));
    assert!(
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty()
    );
}
