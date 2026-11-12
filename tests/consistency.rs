//! S3 / S12：Mock 与 Centralized 的语义一致性。
//!
//! 同一组操作在两个后端上必须返回相同的错误变体或结果形状。

#![allow(clippy::unwrap_used, clippy::expect_used)]

use chain_sdk::config::{
    ChainSdkConfig, ChainSdkMode, DidConfig, ParticipantConfig, SdkRole, build_chain_sdk,
};
use chain_sdk::{
    AnchorType, AssertionPurpose, ChainSdk, ChainSdkError, MockState,
    build_mock_chain_sdk_with_state,
};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn temp_vc_path() -> std::path::PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    std::env::temp_dir().join(format!("chain-sdk-vc-{}-{seq}", std::process::id()))
}

fn config(mode: ChainSdkMode, endpoint: &str) -> ChainSdkConfig {
    ChainSdkConfig {
        mode,
        role: SdkRole::Verifier,
        participant: ParticipantConfig {
            controller_did: "did:web:verifier.example.com".into(),
        },
        did: DidConfig {
            vc_storage_path: temp_vc_path(),
        },
        centralized_endpoint: endpoint.to_string(),
        cache_ttl_secs: 0,
        ..Default::default()
    }
}

fn classify(error: ChainSdkError) -> &'static str {
    match error {
        ChainSdkError::AnchorNotFound(_) => "AnchorNotFound",
        ChainSdkError::ParticipantNotFound(_) => "ParticipantNotFound",
        ChainSdkError::Unauthorized(_) => "Unauthorized",
        ChainSdkError::SignerUnavailable(_) => "SignerUnavailable",
        ChainSdkError::ChainUnavailable(_) => "ChainUnavailable",
        ChainSdkError::NetworkError(_) => "NetworkError",
        ChainSdkError::Unsupported(_) => "Unsupported",
        other => panic!("unexpected error variant: {other:?}"),
    }
}

#[tokio::test]
async fn mock_and_centralized_agree_on_reads_and_errors() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "healthy": true, "block_number": 1 })),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/participants"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&server)
        .await;

    let mock =
        build_mock_chain_sdk_with_state(&config(ChainSdkMode::Mock, ""), MockState::default())
            .unwrap();
    let centralized = build_chain_sdk(&config(ChainSdkMode::Centralized, &server.uri())).unwrap();

    // get_anchor：两边都是 AnchorNotFound。
    assert_eq!(
        classify(mock.get_anchor("missing").await.unwrap_err()),
        "AnchorNotFound"
    );
    assert_eq!(
        classify(centralized.get_anchor("missing").await.unwrap_err()),
        "AnchorNotFound"
    );

    // get_participant：两边都是 ParticipantNotFound。
    assert_eq!(
        classify(mock.get_participant(7).await.unwrap_err()),
        "ParticipantNotFound"
    );
    assert_eq!(
        classify(centralized.get_participant(7).await.unwrap_err()),
        "ParticipantNotFound"
    );

    // by-did：两边都是 None。
    assert!(
        mock.get_participant_by_did("did:web:none")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        centralized
            .get_participant_by_did("did:web:none")
            .await
            .unwrap()
            .is_none()
    );

    // list：两边都是空。
    assert!(mock.list_participants().await.unwrap().is_empty());
    assert!(centralized.list_participants().await.unwrap().is_empty());

    // resolve_did：两边都是 None。
    assert!(mock.resolve_did("did:web:none").await.unwrap().is_none());
    assert!(
        centralized
            .resolve_did("did:web:none")
            .await
            .unwrap()
            .is_none()
    );

    // 健康：两边都健康，模式不同。
    let mock_health = mock.health_check().await.unwrap();
    let centralized_health = centralized.health_check().await.unwrap();
    assert!(mock_health.healthy);
    assert!(centralized_health.healthy);
    assert_eq!(mock_health.mode, "mock");
    assert_eq!(centralized_health.mode, "centralized");
}

#[tokio::test]
async fn mock_and_centralized_agree_on_role_gate() {
    let server = MockServer::start().await;
    let mock =
        build_mock_chain_sdk_with_state(&config(ChainSdkMode::Mock, ""), MockState::default())
            .unwrap();
    let centralized = build_chain_sdk(&config(ChainSdkMode::Centralized, &server.uri())).unwrap();

    let mock_sdk: &dyn ChainSdk = mock.as_ref();
    let centralized_sdk: &dyn ChainSdk = centralized.as_ref();
    for sdk in [mock_sdk, centralized_sdk] {
        assert_eq!(
            classify(
                sdk.create_anchor(AnchorType::EmailBinding, "h", 0)
                    .await
                    .unwrap_err()
            ),
            "Unauthorized"
        );
        assert_eq!(
            classify(
                sdk.sign_participant_assertion(b"p", AssertionPurpose::VcIssuance)
                    .await
                    .unwrap_err()
            ),
            "SignerUnavailable"
        );
    }

    // Centralized 不应因角色拦截而发出任何 HTTP 请求。
    assert!(
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty()
    );
}
