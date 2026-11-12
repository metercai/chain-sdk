//! VC 选择性披露（SD-JWT，`sd-v1`）。
//!
//! 决策依据：`docs/implementation-plan.md` D-13～D-16。
//!
//! - **哈希公式（D-14）**：`H = SHA-256( salt || b64url(name) || b64url(value) )`，
//!   `_sd[i] = b64url_nopad(H)`；与 RFC 9901 的“对 base64url(JSON disclosure) 求哈希”
//!   不同，按附件实现并标记为 `sd-v1`。
//! - **payload_hash（D-15）**：沿用锚定层 `blake2b:` 约定；覆盖“去掉 proof 与
//!   `credentialStatus` 的完整 VC JSON”（`credentialStatus.anchorId` 在锚定之后写入，
//!   由 VC 签名覆盖，不参与锚定哈希，避免先有锚定 id 才能算哈希的循环）。
//! - **自包含信封（D-16）**：`SdCredential` / `SdPresentation` 用 `serde_json::Value`
//!   承载 VC，保留 `credentialStatus`、`_sd` 等 did-sdk 0.1.0 未建模的字段。
//!
//! 端到端交付（Disclosures 存储、用户选择、did-sdk VP 构造）依赖 did-sdk v0.2。

use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::types::AnchorType;
use crate::{ChainSdk, ChainSdkError};

/// `_sd_alg` 支持的唯一算法。
pub const SD_ALG: &str = "sha-256";

/// 一条 Disclosure：`[salt, name, value]`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SdDisclosure {
    /// 16 字节随机盐的 base64url 编码（无 padding）。
    pub salt: String,
    /// 属性名。
    pub name: String,
    /// 属性值（JSON 标量；对象/数组会按其紧凑 JSON 编码）。
    pub value: Value,
}

/// 签发请求。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SdIssueRequest {
    /// VC 类型（追加在 `VerifiableCredential` 之后）。
    pub credential_type: String,
    /// 持有者 DID（`credentialSubject.id`）。
    pub subject_did: String,
    /// 必须披露的属性（明文）。
    pub mandatory_claims: Map<String, Value>,
    /// 可选择性披露的属性。
    pub disclosable_claims: Vec<(String, Value)>,
    /// 链上锚定类型（必须已注册）。
    pub anchor_type: AnchorType,
    /// 锚定有效期（Unix 秒；`0` = 永久）。
    pub valid_until: u64,
    /// VC 过期时间（RFC3339；`None` 表示不过期）。
    pub expiration_date: Option<String>,
}

/// 签发结果：VC 原文 + Disclosures + 锚定信息。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SdIssuedCredential {
    /// 完整 VC JSON（含 `_sd`、`credentialStatus`、`proof`）。
    pub credential: Value,
    /// 全部 Disclosures（交付给持有者）。
    pub disclosures: Vec<SdDisclosure>,
    /// 链上锚定 id。
    pub anchor_id: String,
    /// 锚定载荷哈希（`blake2b:` 前缀）。
    pub payload_hash: String,
}

/// holder 对 VP 的签名。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SdProof {
    /// proof 类型。
    #[serde(rename = "type")]
    pub type_: String,
    /// 创建时间（Unix 秒）。
    pub created: u64,
    /// 验证方法（`{did}#key-1`）。
    #[serde(rename = "verificationMethod")]
    pub verification_method: String,
    /// base58btc 编码的裸 Ed25519 签名。
    pub jws: String,
}

/// 选择性披露 VP（`SD-VP-v1`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SdPresentation {
    /// JSON-LD context。
    #[serde(rename = "@context")]
    pub context: Vec<String>,
    /// 类型列表。
    #[serde(rename = "type")]
    pub types: Vec<String>,
    /// 持有者 DID。
    pub holder: String,
    /// VC 原文。
    #[serde(rename = "verifiableCredential")]
    pub verifiable_credential: Value,
    /// 本次披露的子集。
    pub disclosures: Vec<SdDisclosure>,
    /// 验证方挑战。
    pub challenge: String,
    /// 验证方标识。
    pub domain: String,
    /// holder 签名（覆盖 VC + disclosures + challenge + domain）。
    pub proof: SdProof,
}

/// 验证方期望的会话参数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SdExpected {
    /// 期望的 challenge。
    pub challenge: String,
    /// 期望的 domain。
    pub domain: String,
}

/// 验证结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SdVerificationReport {
    /// 是否全部通过。
    pub verified: bool,
    /// 锚定 id。
    pub anchor_id: String,
    /// 实际披露的属性。
    pub disclosed: BTreeMap<String, Value>,
    /// 未披露属性数量。
    pub undisclosed_count: usize,
}

/// 选择性披露能力（`ChainSdk` 的 supertrait）。
#[async_trait]
pub trait SelectiveDisclosure: Send + Sync {
    /// 签发一张带 `_sd` 的 VC：生成盐化哈希、创建锚定、签名。
    async fn issue_sd_credential(
        &self,
        request: SdIssueRequest,
    ) -> Result<SdIssuedCredential, ChainSdkError>;

    /// 验证 `SD-VP-v1`：holder 签名、VC 签名、Disclosure 哈希、锚定状态。
    async fn verify_sd_presentation(
        &self,
        presentation: &SdPresentation,
        expected: &SdExpected,
    ) -> Result<SdVerificationReport, ChainSdkError>;
}

fn b64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

fn compact(value: &Value) -> Result<Vec<u8>, ChainSdkError> {
    serde_json::to_vec(value)
        .map_err(|error| ChainSdkError::InvalidArgument(format!("compact JSON: {error}")))
}

fn sha256_b64(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    b64(&digest)
}

fn blake2b_hex(bytes: &[u8]) -> String {
    hex::encode(keyvault_core::crypto::blake2::blake2b256(bytes))
}

fn random_salt() -> Result<String, ChainSdkError> {
    let mut raw = [0u8; 16];
    getrandom::fill(&mut raw)
        .map_err(|error| ChainSdkError::SigningFailed(format!("entropy: {error}")))?;
    Ok(b64(&raw))
}

fn scalar_bytes(value: &Value) -> Result<Vec<u8>, ChainSdkError> {
    match value {
        Value::String(text) => Ok(text.as_bytes().to_vec()),
        Value::Bool(_) | Value::Number(_) | Value::Null => compact(value),
        Value::Array(_) | Value::Object(_) => compact(value),
    }
}

/// 计算一条 Disclosure 的 `sd-v1` 哈希。
pub fn disclosure_digest(salt: &str, name: &str, value: &Value) -> Result<String, ChainSdkError> {
    let name = b64(name.as_bytes());
    let value = b64(&scalar_bytes(value)?);
    let input = format!("{salt}{name}{value}");
    Ok(sha256_b64(input.as_bytes()))
}

/// VC 的签名输入（`IssuedVc-v1`；VC 去掉 proof，保留 `credentialStatus`）。
pub fn vc_signing_input(issuer: &str, credential: &Value) -> Result<Vec<u8>, ChainSdkError> {
    let mut unsigned = credential.clone();
    if let Some(object) = unsigned.as_object_mut() {
        object.remove("proof");
    }
    let subject = credential
        .get("credentialSubject")
        .and_then(|subject| subject.get("id"))
        .and_then(Value::as_str)
        .unwrap_or(issuer);
    let credential_type = credential
        .get("type")
        .and_then(Value::as_array)
        .and_then(|types| types.last())
        .and_then(Value::as_str)
        .unwrap_or("VerifiableCredential");
    let hash = blake2b_hex(&compact(&unsigned)?);
    Ok(keyvault_core::signing_input::issued_vc_v1(
        issuer,
        subject,
        credential_type,
        &hash,
    ))
}

/// VP 的签名输入（`SD-VP-v1`；覆盖 VC 与全部 disclosures）。
pub fn vp_signing_input(
    holder: &str,
    challenge: &str,
    domain: &str,
    credential: &Value,
    disclosures: &[SdDisclosure],
) -> Result<Vec<u8>, ChainSdkError> {
    let vc_hash = hex::encode(Sha256::digest(compact(credential)?));
    let mut hashes: Vec<String> = Vec::with_capacity(disclosures.len());
    for disclosure in disclosures {
        let value = serde_json::to_value(disclosure)
            .map_err(|error| ChainSdkError::InvalidArgument(format!("disclosure: {error}")))?;
        hashes.push(hex::encode(Sha256::digest(compact(&value)?)));
    }
    hashes.sort();

    let mut input = format!("SD-VP-v1\n{holder}\n{challenge}\n{domain}\n{vc_hash}");
    for hash in hashes {
        input.push('\n');
        input.push_str(&hash);
    }
    Ok(input.into_bytes())
}

/// 计算锚定用的 `payload_hash`（去掉 proof 与 credentialStatus）。
pub fn payload_hash(credential: &Value) -> Result<String, ChainSdkError> {
    let mut payload = credential.clone();
    if let Some(object) = payload.as_object_mut() {
        object.remove("proof");
        object.remove("credentialStatus");
    }
    Ok(format!("blake2b:{}", blake2b_hex(&compact(&payload)?)))
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

/// Unix 秒 → RFC3339 UTC（`YYYY-MM-DDTHH:MM:SSZ`）。
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

async fn public_key_for<C: ChainSdk + ?Sized>(
    sdk: &C,
    did: &str,
) -> Result<[u8; 32], ChainSdkError> {
    if did.starts_with("did:key:") {
        return Ok(sdk.resolve_did_key(did)?.0);
    }
    if did.starts_with("did:web:") {
        let document = sdk.resolve_did_web(did).await?;
        for method in &document.verification_methods {
            if method.public_key_bytes.len() == 32 {
                let mut key = [0u8; 32];
                key.copy_from_slice(&method.public_key_bytes);
                return Ok(key);
            }
        }
        return Err(ChainSdkError::InvalidArgument(format!(
            "did:web {did} has no Ed25519 verification method"
        )));
    }
    Err(ChainSdkError::Unsupported(format!(
        "unsupported DID method for verification: {did}"
    )))
}

fn decode_signature(jws: &str) -> Result<Vec<u8>, ChainSdkError> {
    bs58::decode(jws)
        .into_vec()
        .map_err(|error| ChainSdkError::InvalidArgument(format!("decode signature: {error}")))
}

/// 签发实现（三后端共享）。
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
        let salt = random_salt()?;
        let digest = disclosure_digest(&salt, name, value)?;
        digests.push(Value::String(digest));
        disclosures.push(SdDisclosure {
            salt,
            name: name.clone(),
            value: value.clone(),
        });
    }
    if !digests.is_empty() {
        subject.insert("_sd".into(), Value::Array(digests));
        subject.insert("_sd_alg".into(), Value::String(SD_ALG.into()));
    } else {
        return Err(ChainSdkError::DisclosureMissing(
            "at least one disclosable claim is required".into(),
        ));
    }

    let mut nonce = [0u8; 16];
    getrandom::fill(&mut nonce)
        .map_err(|error| ChainSdkError::SigningFailed(format!("entropy: {error}")))?;

    let mut credential = Map::new();
    credential.insert(
        "@context".into(),
        Value::Array(vec![Value::String(
            "https://www.w3.org/ns/credentials/v2".into(),
        )]),
    );
    credential.insert(
        "id".into(),
        Value::String(format!("urn:chain-sdk:sd:{}", hex::encode(nonce))),
    );
    credential.insert(
        "type".into(),
        Value::Array(vec![
            Value::String("VerifiableCredential".into()),
            Value::String(request.credential_type.clone()),
        ]),
    );
    credential.insert("issuer".into(), Value::String(issuer.clone()));
    credential.insert("issuanceDate".into(), Value::String(rfc3339_utc(now)));
    if let Some(expiration) = &request.expiration_date {
        credential.insert("expirationDate".into(), Value::String(expiration.clone()));
    }
    credential.insert("credentialSubject".into(), Value::Object(subject));
    let mut credential = Value::Object(credential);

    let payload = payload_hash(&credential)?;
    let anchor_id = sdk
        .create_anchor(request.anchor_type, &payload, request.valid_until)
        .await?;

    if let Some(object) = credential.as_object_mut() {
        object.insert(
            "credentialStatus".into(),
            serde_json::json!({ "type": "AnchorStatus", "anchorId": anchor_id }),
        );
    }

    let signing_input = vc_signing_input(&issuer, &credential)?;
    let signature = sdk
        .sign_participant_assertion(&signing_input, crate::types::AssertionPurpose::VcIssuance)
        .await?;

    if let Some(object) = credential.as_object_mut() {
        object.insert(
            "proof".into(),
            serde_json::json!({
                "type": "Ed25519Signature2020",
                "created": now,
                "verificationMethod": format!("{issuer}#key-1"),
                "jws": bs58::encode(&signature.0).into_string(),
            }),
        );
    }

    Ok(SdIssuedCredential {
        credential,
        disclosures,
        anchor_id,
        payload_hash: payload,
    })
}

/// 验证实现（三后端共享）。
pub(crate) async fn verify<C: ChainSdk + ?Sized>(
    sdk: &C,
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

    let credential = &presentation.verifiable_credential;

    // 1. holder 签名（覆盖 VC + disclosures + challenge + domain）。
    let holder_key = public_key_for(sdk, &presentation.holder).await?;
    let holder_input = vp_signing_input(
        &presentation.holder,
        &presentation.challenge,
        &presentation.domain,
        credential,
        &presentation.disclosures,
    )?;
    let holder_signature = decode_signature(&presentation.proof.jws)?;
    if !keyvault_core::crypto::ed25519::verify(&holder_key, &holder_input, &holder_signature) {
        return Err(ChainSdkError::InvalidArgument(
            "holder signature invalid".into(),
        ));
    }

    // 2. issuer 签名。
    let issuer = credential
        .get("issuer")
        .and_then(Value::as_str)
        .ok_or_else(|| ChainSdkError::InvalidArgument("VC has no issuer".into()))?;
    let issuer_key = public_key_for(sdk, issuer).await?;
    let vc_input = vc_signing_input(issuer, credential)?;
    let jws = credential
        .get("proof")
        .and_then(|proof| proof.get("jws"))
        .and_then(Value::as_str)
        .ok_or_else(|| ChainSdkError::InvalidArgument("VC has no proof".into()))?;
    let issuer_signature = decode_signature(jws)?;
    if !keyvault_core::crypto::ed25519::verify(&issuer_key, &vc_input, &issuer_signature) {
        return Err(ChainSdkError::InvalidArgument(
            "issuer signature invalid".into(),
        ));
    }

    // 3. Disclosure 哈希比对。
    let subject = credential
        .get("credentialSubject")
        .and_then(Value::as_object)
        .ok_or_else(|| ChainSdkError::DisclosureMissing("no credentialSubject".into()))?;
    let alg = subject
        .get("_sd_alg")
        .and_then(Value::as_str)
        .ok_or_else(|| ChainSdkError::SdAlgUnsupported("missing _sd_alg".into()))?;
    if alg != SD_ALG {
        return Err(ChainSdkError::SdAlgUnsupported(alg.to_string()));
    }
    let sd: Vec<&str> = subject
        .get("_sd")
        .and_then(Value::as_array)
        .ok_or_else(|| ChainSdkError::DisclosureMissing("missing _sd array".into()))?
        .iter()
        .filter_map(Value::as_str)
        .collect();

    let mut disclosed = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for disclosure in &presentation.disclosures {
        if !seen.insert(disclosure.name.clone()) {
            return Err(ChainSdkError::DisclosureDuplicate(disclosure.name.clone()));
        }
        let digest = disclosure_digest(&disclosure.salt, &disclosure.name, &disclosure.value)?;
        if !sd.contains(&digest.as_str()) {
            return Err(ChainSdkError::DisclosureHashMismatch);
        }
        disclosed.insert(disclosure.name.clone(), disclosure.value.clone());
    }

    // 4. 锚定检查。
    let anchor_id = credential
        .get("credentialStatus")
        .and_then(|status| status.get("anchorId"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ChainSdkError::DisclosureMissing("VC has no credentialStatus.anchorId".into())
        })?
        .to_string();
    let anchor = sdk.get_anchor(&anchor_id).await?;
    if anchor.revoked {
        return Err(ChainSdkError::AnchorRevoked(anchor_id));
    }
    if anchor.valid_until != 0 && unix_now() > anchor.valid_until {
        return Err(ChainSdkError::AnchorExpired(anchor_id));
    }
    let recomputed = payload_hash(credential)?;
    if recomputed != anchor.payload_hash {
        return Err(ChainSdkError::AnchorPayloadMismatch(format!(
            "expected {}, recomputed {recomputed}",
            anchor.payload_hash
        )));
    }

    let undisclosed_count = sd.len().saturating_sub(disclosed.len());
    Ok(SdVerificationReport {
        verified: true,
        anchor_id,
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
        verify(self, presentation, expected).await
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
        verify(self, presentation, expected).await
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
        verify(self, presentation, expected).await
    }
}
