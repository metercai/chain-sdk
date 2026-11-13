//! CS-06: reproducible Centralized write-signature vectors.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use serde_json::Value;

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/centralized_wire.json")).unwrap()
}

#[test]
fn wire_vectors_are_canonical_and_signed() {
    let fixture = fixture();
    let signer = fixture["signer"].as_str().unwrap();
    assert!(signer.starts_with("chain-account/"), "{signer}");

    let public: [u8; 32] = hex::decode(fixture["public_key_hex"].as_str().unwrap())
        .unwrap()
        .try_into()
        .expect("32-byte sr25519 public key");

    let vectors = fixture["vectors"].as_array().unwrap();
    assert_eq!(vectors.len(), 3);
    for vector in vectors {
        let op = vector["op"].as_str().unwrap();
        let body = &vector["body"];
        let canonical = String::from_utf8(chain_sdk::canonical_json(body).unwrap()).unwrap();
        assert_eq!(canonical, vector["canonical"].as_str().unwrap(), "{op}");
        assert_eq!(
            vector["payload"].as_str().unwrap(),
            format!("{op}\n{canonical}"),
            "{op}"
        );

        let signature = hex::decode(vector["signature_hex"].as_str().unwrap()).unwrap();
        assert_eq!(signature.len(), 64);
        assert!(
            keyvault_core::crypto::sr25519::verify(
                &public,
                vector["payload"].as_str().unwrap().as_bytes(),
                &signature
            ),
            "{op} signature must verify"
        );
    }
}
