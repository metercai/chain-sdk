//! VC-native salted-hash selective disclosure on top of `did-sdk` v0.2.
//!
//! Contract (see `docs/ChainSdk-Interface-Contract.md`):
//!
//! - **Hashing (sd-v1)**: `SHA-256( salt || b64url(name) || b64url(value) )`,
//!   `_sd[i] = b64url_nopad(hash)`. This is the same formula used by
//!   `did-sdk` / `keyvault-core` v0.2 (`Disclosure::hash_string`).
//! - **VC** is `did_sdk::VerifiableCredential`; the on-chain anchor id is
//!   stored in `anchor_id` (not `credentialStatus`).
//! - **VP** is `did_sdk::VerifiablePresentation` with `disclosures: Some(..)`
//!   and is verified with `DidSdk::<Server>::verify_vp` (`VP-SD-v1` signing
//!   input, including holder signature, challenge and VC signatures).
//! - **Anchor**: `payload_hash` uses the anchor-layer BLAKE2b convention over
//!   the VC without `proof` / `anchor_id` (the id does not exist yet when the
//!   anchor is created; it is covered by the issuer signature instead).

use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use keyvault_core::signing_input::{issued_vc_v1, vp_sd_v1};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::did_adapter::DidPort;
use crate::types::AnchorType;
use crate::{ChainSdk, ChainSdkError};

/// Re-export of `did_sdk::Disclosure` (`[salt, name, value]`).
pub use did_sdk::Disclosure as SdDisclosure;
/// Re-export of `did_sdk::VerifiablePresentation` (`disclosures` optional).
pub type SdPresentation = did_sdk::VerifiablePresentation;

/// `_sd_alg` value supported by this crate.
pub const SD_ALG: &str = "sha-256";

/// Issuance request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SdIssueRequest {
    /// VC type (appended after `VerifiableCredential`).
    pub credential_type: String,
    /// Holder DID (`credentialSubject.id`).
    pub subject_did: String,
    /// Claims that are always disclosed (plaintext).
    pub mandatory_claims: Map<String, Value>,
    /// Claims that can be selectively disclosed.
    pub disclosable_claims: Vec<(String, Value)>,
    /// Anchor type (must be registered on chain).
    pub anchor_type: AnchorType,
    /// Anchor validity (Unix seconds; `0` = permanent).
    pub valid_until: u64,
    /// VC expiration (RFC3339; `None` = no expiration).
    pub expiration_date: Option<String>,
}

/// Issuance result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SdIssuedCredential {
    /// VC with `_sd`, `anchor_id` and `proof`.
    pub credential: did_sdk::VerifiableCredential,
    /// All disclosures (delivered to the holder).
    pub disclosures: Vec<SdDisclosure>,
    /// On-chain anchor id.
    pub anchor_id: String,
    /// Anchor payload hash (`blake2b:` prefix).
    pub payload_hash: String,
}

/// Session parameters expected by the verifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SdExpected {
    /// Expected challenge.
    pub challenge: String,
    /// Expected domain.
    pub domain: String,
}

/// Verification result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SdVerificationReport {
    /// Whether all checks passed.
    pub verified: bool,
    /// Anchor id of the first credential.
    pub anchor_id: String,
    /// Disclosed attributes.
    pub disclosed: BTreeMap<String, Value>,
    /// Number of attributes that were not disclosed.
    pub undisclosed_count: usize,
}

/// Selective disclosure capability (`ChainSdk` supertrait).
#[async_trait]
pub trait SelectiveDisclosure: Send + Sync {
    /// Issue an SD credential: salted hashes, anchor, issuer proof.
    async fn issue_sd_credential(
        &self,
        request: SdIssueRequest,
    ) -> Result<SdIssuedCredential, ChainSdkError>;

    /// Verify an SD presentation (holder/VC proofs, disclosures, anchor).
    async fn verify_sd_presentation(
        &self,
        presentation: &SdPresentation,
        expected: &SdExpected,
    ) -> Result<SdVerificationReport, ChainSdkError>;
}

/// Compute a disclosure digest (same as `did-sdk` v0.2).
pub fn disclosure_digest(disclosure: &SdDisclosure) -> Result<String, ChainSdkError> {
    disclosure
        .hash_string()
        .map_err(crate::did_adapter::map_did_err)
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn compact<T: Serialize>(value: &T) -> Result<Vec<u8>, ChainSdkError> {
    serde_json::to_vec(value)
        .map_err(|error| ChainSdkError::InvalidArgument(format!("compact JSON: {error}")))
}

/// `payload_hash` for anchoring: VC without `proof` and `anchor_id`.
///
/// Delegates to `did_sdk::anchor_payload_hash` so the two SDKs cannot drift.
pub fn payload_hash(credential: &did_sdk::VerifiableCredential) -> Result<String, ChainSdkError> {
    did_sdk::anchor_payload_hash(credential).map_err(crate::did_adapter::map_did_err)
}

/// `IssuedVc-v1` signing input (must match `did-sdk` `verify_vc`).
pub fn vc_signing_input(
    credential: &did_sdk::VerifiableCredential,
) -> Result<Vec<u8>, ChainSdkError> {
    let hash =
        did_sdk::vc_hash_without_proof(credential).map_err(crate::did_adapter::map_did_err)?;
    let subject = credential
        .credential_subject
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or(&credential.issuer);
    let credential_type = credential
        .types
        .last()
        .map(String::as_str)
        .unwrap_or("VerifiableCredential");
    Ok(issued_vc_v1(
        &credential.issuer,
        subject,
        credential_type,
        &hash,
    ))
}

/// `VP-SD-v1` signing input (must match `did-sdk` v0.2).
pub fn vp_signing_input(
    holder: &str,
    challenge: &str,
    domain: &str,
    credentials: &[did_sdk::VerifiableCredential],
    disclosures: &[SdDisclosure],
) -> Result<Vec<u8>, ChainSdkError> {
    let disclosures_hash = sha256_hex(&compact(&disclosures.to_vec())?);
    let mut vc_hashes = Vec::with_capacity(credentials.len());
    for credential in credentials {
        vc_hashes.push(did_sdk::vc_hash(credential).map_err(crate::did_adapter::map_did_err)?);
    }
    Ok(vp_sd_v1(
        holder,
        challenge,
        domain,
        &disclosures_hash,
        &vc_hashes,
    ))
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

/// Unix seconds → RFC3339 UTC.
pub fn rfc3339_utc(timestamp: u64) -> String {
    let days = (timestamp / 86_400) as i64;
    let seconds = timestamp % 86_400;
    let hour = seconds / 3_600;
    let minute = (seconds % 3_600) / 60;
    let second = seconds % 60;

    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = if month <= 2 { year + 1 } else { year };

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Shared issuance implementation.
pub(crate) async fn issue<C: ChainSdk + ?Sized>(
    sdk: &C,
    request: SdIssueRequest,
) -> Result<SdIssuedCredential, ChainSdkError> {
    let issuer = sdk.participant_did().to_string();
    if issuer.trim().is_empty() {
        return Err(ChainSdkError::ConfigError(
            "participant.controller_did is required for SD issuance".into(),
        ));
    }
    let now = unix_now();

    let mut subject = Map::new();
    subject.insert("id".into(), Value::String(request.subject_did.clone()));
    for (key, value) in &request.mandatory_claims {
        if key != "id" && key != "_sd" && key != "_sd_alg" {
            subject.insert(key.clone(), value.clone());
        }
    }

    let mut disclosures = Vec::with_capacity(request.disclosable_claims.len());
    let mut digests = Vec::with_capacity(request.disclosable_claims.len());
    let mut names = BTreeSet::new();
    for (name, value) in &request.disclosable_claims {
        if !names.insert(name.clone()) {
            return Err(ChainSdkError::DisclosureDuplicate(name.clone()));
        }
        let disclosure = SdDisclosure::generate(name.clone(), value.clone())
            .map_err(crate::did_adapter::map_did_err)?;
        let digest = disclosure_digest(&disclosure)?;
        digests.push(Value::String(digest));
        disclosures.push(disclosure);
    }
    if digests.is_empty() {
        return Err(ChainSdkError::DisclosureMissing(
            "at least one disclosable claim is required".into(),
        ));
    }
    subject.insert("_sd".into(), Value::Array(digests));
    subject.insert("_sd_alg".into(), Value::String(SD_ALG.into()));

    let mut nonce = [0u8; 16];
    getrandom::fill(&mut nonce)
        .map_err(|error| ChainSdkError::SigningFailed(format!("entropy: {error}")))?;

    let mut credential = did_sdk::VerifiableCredential {
        context: vec!["https://www.w3.org/ns/credentials/v2".into()],
        id: format!("urn:chain-sdk:sd:{}", hex::encode(nonce)),
        types: vec![
            "VerifiableCredential".into(),
            request.credential_type.clone(),
        ],
        issuer: issuer.clone(),
        issuance_date: rfc3339_utc(now),
        expiration_date: request.expiration_date.clone(),
        credential_subject: Value::Object(subject),
        proof: None,
        anchor_id: None,
    };

    let payload = payload_hash(&credential)?;
    let anchor_id = sdk
        .create_anchor(request.anchor_type, &payload, request.valid_until)
        .await?;
    credential.anchor_id = Some(anchor_id.clone());

    let signing_input = vc_signing_input(&credential)?;
    let signature = sdk
        .sign_participant_assertion(&signing_input, crate::types::AssertionPurpose::VcIssuance)
        .await?;
    credential.proof = Some(did_sdk::Proof {
        type_: "Ed25519Signature2020".into(),
        created: now,
        verification_method: format!("{issuer}#key-1"),
        jws: bs58::encode(&signature.0).into_string(),
    });

    Ok(SdIssuedCredential {
        credential,
        disclosures,
        anchor_id,
        payload_hash: payload,
    })
}

/// Shared verification implementation.
pub(crate) async fn verify<C: ChainSdk + ?Sized>(
    sdk: &C,
    did: &dyn DidPort,
    presentation: &SdPresentation,
    expected: &SdExpected,
) -> Result<SdVerificationReport, ChainSdkError> {
    if presentation.challenge != expected.challenge {
        return Err(ChainSdkError::Unauthorized(
            "challenge does not match this session".into(),
        ));
    }
    if presentation.domain != expected.domain {
        return Err(ChainSdkError::Unauthorized(
            "domain does not match this verifier".into(),
        ));
    }
    let Some(disclosures) = presentation.disclosures.as_ref() else {
        return Err(ChainSdkError::DisclosureMissing(
            "VP has no disclosures (non-SD presentation)".into(),
        ));
    };
    if presentation.proof.is_none() {
        return Err(ChainSdkError::InvalidArgument(
            "VP has no holder proof".into(),
        ));
    }
    if presentation.verifiable_credential.is_empty() {
        return Err(ChainSdkError::InvalidArgument(
            "VP has no credential".into(),
        ));
    }

    // holder signature + challenge + VC signatures + disclosure hashes
    let outcome = did.verify_vp(presentation, &expected.challenge).await?;
    if !outcome.verified {
        return Err(ChainSdkError::InvalidArgument(format!(
            "VP verification failed: {}",
            outcome.reason.unwrap_or_default()
        )));
    }

    let mut disclosed = BTreeMap::new();
    for disclosure in disclosures {
        if disclosed
            .insert(disclosure.name.clone(), disclosure.value.clone())
            .is_some()
        {
            return Err(ChainSdkError::DisclosureDuplicate(disclosure.name.clone()));
        }
    }

    let mut anchor_id = None;
    let mut undisclosed_count = 0usize;
    for credential in &presentation.verifiable_credential {
        let id = credential
            .anchor_id
            .clone()
            .ok_or_else(|| ChainSdkError::DisclosureMissing("VC has no anchor_id".into()))?;
        if anchor_id.is_none() {
            anchor_id = Some(id.clone());
        }
        let anchor = sdk.get_anchor(&id).await?;
        if anchor.revoked {
            return Err(ChainSdkError::AnchorRevoked(id));
        }
        if anchor.valid_until != 0 && unix_now() > anchor.valid_until {
            return Err(ChainSdkError::AnchorExpired(id));
        }
        let recomputed = payload_hash(credential)?;
        if recomputed != anchor.payload_hash {
            return Err(ChainSdkError::AnchorPayloadMismatch(format!(
                "expected {}, recomputed {recomputed}",
                anchor.payload_hash
            )));
        }
        let sd_count = credential
            .credential_subject
            .get("_sd")
            .and_then(Value::as_array)
            .map(|array| array.len())
            .unwrap_or(0);
        undisclosed_count += sd_count;
    }
    undisclosed_count = undisclosed_count.saturating_sub(disclosed.len());

    Ok(SdVerificationReport {
        verified: true,
        anchor_id: anchor_id.unwrap_or_default(),
        disclosed,
        undisclosed_count,
    })
}

#[async_trait]
impl SelectiveDisclosure for crate::mock::MockChainSdk {
    async fn issue_sd_credential(
        &self,
        request: SdIssueRequest,
    ) -> Result<SdIssuedCredential, ChainSdkError> {
        issue(self, request).await
    }

    async fn verify_sd_presentation(
        &self,
        presentation: &SdPresentation,
        expected: &SdExpected,
    ) -> Result<SdVerificationReport, ChainSdkError> {
        let did = self
            .core()
            .did
            .as_deref()
            .ok_or_else(|| ChainSdkError::SignerUnavailable("did-sdk not assembled".into()))?;
        verify(self, did, presentation, expected).await
    }
}

#[async_trait]
impl SelectiveDisclosure for crate::centralized::CentralizedChainSdk {
    async fn issue_sd_credential(
        &self,
        request: SdIssueRequest,
    ) -> Result<SdIssuedCredential, ChainSdkError> {
        issue(self, request).await
    }

    async fn verify_sd_presentation(
        &self,
        presentation: &SdPresentation,
        expected: &SdExpected,
    ) -> Result<SdVerificationReport, ChainSdkError> {
        let did = self
            .core()
            .did
            .as_deref()
            .ok_or_else(|| ChainSdkError::SignerUnavailable("did-sdk not assembled".into()))?;
        verify(self, did, presentation, expected).await
    }
}

#[async_trait]
impl SelectiveDisclosure for crate::chain::ChainModeSdk {
    async fn issue_sd_credential(
        &self,
        request: SdIssueRequest,
    ) -> Result<SdIssuedCredential, ChainSdkError> {
        issue(self, request).await
    }

    async fn verify_sd_presentation(
        &self,
        presentation: &SdPresentation,
        expected: &SdExpected,
    ) -> Result<SdVerificationReport, ChainSdkError> {
        let did = self
            .core()
            .did
            .as_deref()
            .ok_or_else(|| ChainSdkError::SignerUnavailable("did-sdk not assembled".into()))?;
        verify(self, did, presentation, expected).await
    }
}
