//! CS-05: byte-for-byte parity with `did-sdk` v0.2.1 hash APIs.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use chain_sdk::sd::{SdDisclosure, payload_hash, vc_signing_input, vp_signing_input};
use did_sdk::VerifiableCredential;
use serde_json::json;
use sha2::{Digest, Sha256};

fn credential() -> VerifiableCredential {
    VerifiableCredential {
        context: vec!["https://www.w3.org/ns/credentials/v2".into()],
        id: "urn:example:vc:1".into(),
        types: vec!["VerifiableCredential".into(), "IDCardVC".into()],
        issuer: "did:key:zIssuer".into(),
        issuance_date: "2026-01-01T00:00:00Z".into(),
        expiration_date: Some("2036-01-01T00:00:00Z".into()),
        credential_subject: json!({
            "id": "did:key:zHolder",
            "idCardNumber": "110101199001011234",
            "_sd": ["hash-1", "hash-2"],
            "_sd_alg": "sha-256"
        }),
        proof: Some(did_sdk::Proof {
            type_: "Ed25519Signature2020".into(),
            created: 1,
            verification_method: "did:key:zIssuer#key-1".into(),
            jws: "zSig".into(),
        }),
        anchor_id: Some("anchor:1".into()),
    }
}

#[test]
fn payload_hash_matches_did_sdk() {
    let vc = credential();
    assert_eq!(
        payload_hash(&vc).unwrap(),
        did_sdk::anchor_payload_hash(&vc).unwrap()
    );
}

#[test]
fn vc_signing_input_matches_did_sdk_hash() {
    let vc = credential();
    let expected = keyvault_core::signing_input::issued_vc_v1(
        &vc.issuer,
        "did:key:zHolder",
        "IDCardVC",
        &did_sdk::vc_hash_without_proof(&vc).unwrap(),
    );
    assert_eq!(vc_signing_input(&vc).unwrap(), expected);
}

#[test]
fn vp_signing_input_matches_did_sdk_hashes() {
    let vc = credential();
    let disclosures = vec![SdDisclosure {
        salt: "AAECAwQFBgcICQoLDA0ODw".into(),
        name: "name".into(),
        value: json!("Alice"),
    }];
    let disclosures_hash = hex::encode(Sha256::digest(serde_json::to_vec(&disclosures).unwrap()));
    let expected = keyvault_core::signing_input::vp_sd_v1(
        "did:key:zHolder",
        "challenge",
        "verifier.example.com",
        &disclosures_hash,
        &[did_sdk::vc_hash(&vc).unwrap()],
    );
    assert_eq!(
        vp_signing_input(
            "did:key:zHolder",
            "challenge",
            "verifier.example.com",
            std::slice::from_ref(&vc),
            &disclosures
        )
        .unwrap(),
        expected
    );
}

#[test]
fn disclosure_digest_matches_did_sdk() {
    let disclosure = SdDisclosure {
        salt: "AAECAwQFBgcICQoLDA0ODw".into(),
        name: "name".into(),
        value: json!("Alice"),
    };
    assert_eq!(
        chain_sdk::sd::disclosure_digest(&disclosure).unwrap(),
        disclosure.hash_string().unwrap()
    );
    assert_eq!(
        chain_sdk::sd::disclosure_digest(&disclosure).unwrap(),
        "XGuolHxMRBLapsPiERv_dGyHZObbvA3efbPCxjsfbZA"
    );
}
