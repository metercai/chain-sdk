//! S2 / S3-AppAlias：配置默认值、env、override 优先级与 `role = "app"` 别名。
//!
//! 所有用例都通过 `temp_env` 隔离环境变量，并把 `CHAIN_SDK_HOME` 指向临时目录，
//! 避免在开发者机器上写入密钥/密码文件。

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use chain_sdk::ChainSdkError;
use chain_sdk::config::{ChainSdkConfig, ChainSdkMode, ConfigOverrides, SdkRole, resolve_config};

/// 需要在每个用例中清理的环境变量（避免宿主机环境影响断言）。
const CLEARED: &[&str] = &[
    "CHAIN_SDK_MODE",
    "CHAIN_SDK_ROLE",
    "CHAIN_SDK_PARTICIPANT_ID",
    "CHAIN_SDK_CONTROLLER_DID",
    "CHAIN_SDK_KEYVAULT_SCHEME",
    "CHAIN_SDK_KEYVAULT_PATH",
    "CHAIN_SDK_KEYVAULT_PASSWORD_FILE",
    "CHAIN_SDK_OPERATOR_KEY_ID",
    "CHAIN_SDK_PARTICIPANT_DID_KEY_ID",
    "CHAIN_SDK_VC_WALLET",
    "CHAIN_SDK_ISSUER_SIGNING",
    "CHAIN_SDK_CENTRALIZED_ENDPOINT",
    "CHAIN_SDK_CHAIN_ENDPOINT",
    "CHAIN_SDK_REQUIRE_FINALIZED",
    "CHAIN_SDK_CACHE_TTL_SECS",
    "CHAIN_SDK_DID_VC_STORAGE_PATH",
    "CHAIN_SDK_MOCK_STATE_PATH",
    "CHAIN_KEYSTORE_PASSWORD",
];

fn with_clean_env<R>(
    home: &Path,
    config_file: Option<&Path>,
    extra: &[(&'static str, Option<&'static str>)],
    f: impl FnOnce() -> R,
) -> R {
    let mut vars: Vec<(&str, Option<&OsStr>)> = CLEARED.iter().map(|name| (*name, None)).collect();
    vars.push(("CHAIN_SDK_HOME", Some(home.as_os_str())));
    vars.push((
        "CHAIN_SDK_CONFIG_FILE",
        config_file.map(|path| path.as_os_str()),
    ));
    for (key, value) in extra {
        vars.push((key, value.map(OsStr::new)));
    }
    temp_env::with_vars(vars, f)
}

fn fixed_home() -> PathBuf {
    std::env::temp_dir().join("chain-sdk-config-tests")
}

#[test]
fn default_resolution_is_mock_verifier() {
    let home = fixed_home();
    with_clean_env(&home, None, &[], || {
        let config = resolve_config(ConfigOverrides::default()).unwrap();
        assert_eq!(config.mode, ChainSdkMode::Mock);
        assert_eq!(config.role, SdkRole::Verifier);
        assert_eq!(config.participant_id, 0);
        assert_eq!(config.cache_ttl_secs, 300);
        assert!(config.capabilities.did_resolution);
        assert!(!config.capabilities.issuer_signing);
        // Verifier 不解析密码（S17）。
        assert!(config.resolved_password.is_none());
    });
}

#[test]
fn role_app_alias_maps_to_issuer() {
    let config: ChainSdkConfig = toml::from_str("mode = \"mock\"\nrole = \"app\"\n").unwrap();
    assert_eq!(config.role, SdkRole::Issuer);

    let config: ChainSdkConfig = toml::from_str("role = \"issuer\"").unwrap();
    assert_eq!(config.role, SdkRole::Issuer);

    let config: ChainSdkConfig = toml::from_str("role = \"verifier\"").unwrap();
    assert_eq!(config.role, SdkRole::Verifier);
}

#[test]
fn verifier_does_not_resolve_password() {
    let home = fixed_home();
    with_clean_env(
        &home,
        None,
        &[
            ("CHAIN_SDK_ROLE", Some("verifier")),
            ("CHAIN_SDK_KEYVAULT_SCHEME", Some("file-keystore")),
        ],
        || {
            let config = resolve_config(ConfigOverrides::default()).unwrap();
            assert!(config.resolved_password.is_none());
        },
    );
}

#[test]
fn issuer_with_memory_keystore_needs_no_password() {
    let home = fixed_home();
    with_clean_env(
        &home,
        None,
        &[
            ("CHAIN_SDK_ROLE", Some("issuer")),
            ("CHAIN_SDK_KEYVAULT_SCHEME", Some("memory-keystore")),
            (
                "CHAIN_SDK_CONTROLLER_DID",
                Some("did:web:mail.example.com:issuer"),
            ),
            ("CHAIN_SDK_ISSUER_SIGNING", Some("true")),
        ],
        || {
            let config = resolve_config(ConfigOverrides::default()).unwrap();
            assert_eq!(config.role, SdkRole::Issuer);
            assert!(config.resolved_password.is_none());
            assert!(config.capabilities.issuer_signing);
        },
    );
}

#[test]
fn verifier_forces_issuer_signing_off() {
    let home = fixed_home();
    with_clean_env(
        &home,
        None,
        &[
            ("CHAIN_SDK_ROLE", Some("verifier")),
            ("CHAIN_SDK_ISSUER_SIGNING", Some("true")),
        ],
        || {
            let config = resolve_config(ConfigOverrides::default()).unwrap();
            assert!(!config.capabilities.issuer_signing);
        },
    );
}

#[test]
fn file_config_is_parsed_for_issuer_and_verifier() {
    let dir = tempfile::tempdir().unwrap();
    let issuer_path = dir.path().join("issuer.toml");
    std::fs::write(&issuer_path, include_str!("configs/issuer.toml")).unwrap();

    with_clean_env(dir.path(), Some(&issuer_path), &[], || {
        let config = resolve_config(ConfigOverrides::default()).unwrap();
        assert_eq!(config.mode, ChainSdkMode::Mock);
        assert_eq!(config.role, SdkRole::Issuer);
        assert_eq!(config.participant_id, 4);
        assert_eq!(
            config.participant.controller_did,
            "did:web:mail.example.com:issuer"
        );
        assert_eq!(config.key_vault.scheme, "memory-keystore");
        assert_eq!(config.key_vault.keys.operator, "chain-account/operator-1");
        assert_eq!(
            config.key_vault.keys.participant_did,
            "participant-did/did_web_mail_example_com"
        );
        assert!(config.capabilities.issuer_signing);
        assert_eq!(config.chain_endpoint, "ws://127.0.0.1:9944");
        assert_eq!(
            config.did.vc_storage_path,
            std::path::PathBuf::from("/tmp/unused-vc-store")
        );
    });

    let verifier_path = dir.path().join("verifier.toml");
    std::fs::write(&verifier_path, include_str!("configs/verifier.toml")).unwrap();
    with_clean_env(dir.path(), Some(&verifier_path), &[], || {
        let config = resolve_config(ConfigOverrides::default()).unwrap();
        assert_eq!(config.role, SdkRole::Verifier);
        assert_eq!(config.mode, ChainSdkMode::Mock);
        assert!(!config.capabilities.issuer_signing);
        assert!(config.resolved_password.is_none());
    });
}

#[test]
fn env_overrides_file_and_explicit_overrides_win() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("verifier.toml");
    std::fs::write(&config_path, include_str!("configs/verifier.toml")).unwrap();

    with_clean_env(
        dir.path(),
        Some(&config_path),
        &[
            ("CHAIN_SDK_MODE", Some("centralized")),
            ("CHAIN_SDK_PARTICIPANT_ID", Some("9")),
        ],
        || {
            let config = resolve_config(ConfigOverrides {
                participant_id: Some(42),
                ..Default::default()
            })
            .unwrap();
            // env > file
            assert_eq!(config.mode, ChainSdkMode::Centralized);
            // override > env
            assert_eq!(config.participant_id, 42);
            // file 仍生效
            assert_eq!(config.role, SdkRole::Verifier);
        },
    );
}

#[test]
fn invalid_mode_and_role_are_config_errors() {
    let home = fixed_home();
    with_clean_env(&home, None, &[], || {
        let error = resolve_config(ConfigOverrides {
            mode: Some("bogus".into()),
            ..Default::default()
        })
        .unwrap_err();
        assert!(matches!(error, ChainSdkError::ConfigError(_)));

        let error = resolve_config(ConfigOverrides {
            role: Some("owner".into()),
            ..Default::default()
        })
        .unwrap_err();
        assert!(matches!(error, ChainSdkError::ConfigError(_)));
    });
}

#[test]
fn issuer_requires_controller_did() {
    let home = fixed_home();
    with_clean_env(
        &home,
        None,
        &[
            ("CHAIN_SDK_ROLE", Some("issuer")),
            ("CHAIN_SDK_KEYVAULT_SCHEME", Some("memory-keystore")),
        ],
        || {
            let error = resolve_config(ConfigOverrides::default()).unwrap_err();
            match error {
                ChainSdkError::ConfigError(message) => {
                    assert!(message.contains("controller_did"), "{message}");
                }
                other => panic!("unexpected error: {other:?}"),
            }
        },
    );
}

#[test]
fn cache_ttl_can_be_disabled_with_zero() {
    let home = fixed_home();
    with_clean_env(&home, None, &[], || {
        let config = resolve_config(ConfigOverrides {
            cache_ttl_secs: Some(0),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(config.cache_ttl_secs, 0);
    });
}
