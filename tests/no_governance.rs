//! S18：`ChainSdk` trait 不得包含治理方法；VC 存储方法也不得出现在本 crate。
//!
//! 通过 `include_str!` 在编译期读取 `src/lib.rs`，做源码级扫描；CI 另有
//! `scripts/verify-roles.sh` 对 `src/` 全目录 grep。

#![allow(clippy::unwrap_used, clippy::expect_used)]

const LIB_SOURCE: &str = include_str!("../src/lib.rs");

const EXPECTED_METHODS: &[&str] = &[
    "participant_did",
    "health_check",
    "get_anchor",
    "list_anchors",
    "get_participant",
    "get_participant_by_did",
    "list_participants",
    "resolve_did",
    "resolve_did_key",
    "resolve_did_web",
    "create_anchor",
    "revoke_anchor",
    "renew_anchor",
    "update_authorized_domains",
    "update_controller_did",
    "sign_participant_assertion",
    "subscribe_event",
];

const FORBIDDEN_GOVERNANCE: &[&str] = &[
    "register_participant",
    "deactivate_participant",
    "update_operator",
    "register_anchor_type",
    "submit_delegated_anchor",
];

const FORBIDDEN_VC_STORAGE: &[&str] = &["store_vc", "get_vc", "list_vcs", "remove_vc"];

#[test]
fn trait_contains_all_expected_methods() {
    assert!(
        LIB_SOURCE.contains("pub trait ChainSdk"),
        "ChainSdk trait must be declared in src/lib.rs"
    );
    for method in EXPECTED_METHODS {
        let needle = format!("fn {method}");
        assert!(
            LIB_SOURCE.contains(&needle),
            "missing expected trait method: {method}"
        );
    }
}

#[test]
fn trait_has_no_governance_methods() {
    for method in FORBIDDEN_GOVERNANCE {
        let needle = format!("fn {method}");
        assert!(
            !LIB_SOURCE.contains(&needle),
            "governance method {method} must live in chain-owner-sdk (S18)"
        );
    }
}

#[test]
fn trait_has_no_vc_storage_methods() {
    for method in FORBIDDEN_VC_STORAGE {
        let needle = format!("fn {method}");
        assert!(
            !LIB_SOURCE.contains(&needle),
            "VC storage method {method} must live in did-sdk (S17)"
        );
    }
}
