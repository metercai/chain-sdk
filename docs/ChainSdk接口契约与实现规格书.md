# ChainSdk Interface Contract and Implementation Specification

| Item | Value |
|---|---|
| Document | ChainSdk interface contract and implementation specification (initial release) |
| Package | `chain-sdk` v0.1.0 |
| Status | Implemented and released; 96 tests passing; CI green |
| Audience | Service-side Rust applications (gateways, issuers, verifiers), `chain-owner-sdk`, `did-sdk` maintainers |
| Normative | This document is the single source of truth for the contract. Any change follows §13. |

---

## 1. Scope

### 1.1 Purpose

`chain-sdk` is the **chain-capability contract** for service-side Rust applications. It provides
one trait, `ChainSdk`, implemented by three backends and parameterised by two service roles:

- backends: `Mock` (in-process), `Centralized` (HTTP + WebSocket), `Chain` (Substrate via `subxt`);
- roles: `Verifier` (read-only) and `Issuer` (read + write + assertion signing).

It owns anchor queries and writes, participant queries, DID resolution entry points, participant
assertion signing and event subscriptions. It is not a wallet and not a governance SDK.

### 1.2 Responsibilities and boundaries

```text
chain-owner-sdk          governance: register_participant, update_operator,
                         register_anchor_type, submit_delegated_anchor
        │ depends on
        ▼
chain-sdk                anchor read/write, participant read, DID resolution,
                         assertion signing, events, SD-JWT issuance/verification
        │ depends on
        ├── did-sdk v0.2        DID methods + VC wallet + SD disclosures (no chain access)
        └── keyvault-core v0.2  KeyVault trait, FileKeystore, crypto primitives
```

| Concern | Owner |
|---|---|
| Anchor queries (`get_anchor`, `list_anchors`) | **chain-sdk** (verifier responsibility) |
| Anchor writes, participant self-management | **chain-sdk** (`Issuer`) |
| Participant registration / operator rotation / anchor-type registration / delegated-anchor submission | `chain-owner-sdk` |
| VC storage, disclosure storage, holder presentation (`VP`) | `did-sdk<Server>` / `did-sdk<Client>` |
| Private keys | `keyvault-core` (never exported) |
| `subxt` | **chain-sdk only** |

**R1 boundary (normative).** `did-sdk` has no `Chain` backend, no `get_anchor` and no `subxt`
dependency. `chain-sdk` must not expose governance methods or VC storage methods.

### 1.3 Non-goals

- Python / Node / WASM bindings. `chain-sdk` is a server-side Rust contract.
- Production HSM / device secure enclave (feature hooks only).
- BBS+ selective disclosure (see §7.8).
- Real-chain acceptance without chain metadata and a node (see §5.3).

---

## 2. Roles, Backends and Capabilities

### 2.1 Service role: `SdkRole`

| Variant | Capabilities | Keys |
|---|---|---|
| `Verifier` | reads only (anchors, participants, DIDs, events) | **no** `KeyVault` loaded |
| `Issuer` | all `Verifier` reads + anchor writes + self-management + assertion signing | `chain-account/` (sr25519) and `participant-did/` (ed25519) |

`Verifier ⊂ Issuer` (linear inclusion). There is no "Issuer that cannot verify".

`role = "app"` is accepted on input as an alias of `issuer` for backward compatibility. It is not a
separate variant.

### 2.2 On-chain role: `ChainRole`

`ParticipantRecord.chain_role` is the **on-chain registry field** assigned by chain governance:
`Verifier`, `Issuer` or `ChainOwner`. It describes the on-chain capability boundary.

`SdkRole` is the **local instantiation choice** and decides which keys and capabilities are loaded.
The two concepts must not be merged.

### 2.3 Backend matrix

| Backend | `Verifier` | `Issuer` |
|---|---|---|
| `Mock` | in-memory reads; no `KeyVault`; assertion signing → `SignerUnavailable` | in-memory read/write; two keys |
| `Centralized` | HTTP reads; no `KeyVault` | HTTP writes signed with the chain key; `did:web` assertion signing |
| `Chain` | `subxt` storage reads (POC: `Unsupported`, §5.3); no `KeyVault` | `subxt` writes (POC: `Unsupported` after role gate); sr25519 extrinsic signing + ed25519 assertion signing |

Privilege violations are identical across backends:

| Call | `Verifier` | `Issuer` |
|---|---|---|
| write methods (`create_anchor`, …) | `Unauthorized` | allowed (Chain POC: `Unsupported`) |
| `update_*` on another participant | `Unauthorized` | `Unauthorized` |
| `sign_participant_assertion` | `SignerUnavailable` | allowed |

### 2.4 Capability assembly (did-sdk subset)

| Capability | `Verifier` | `Issuer` | Config |
|---|---|---|---|
| `did-resolution` | yes | yes | `capabilities.did_resolution` |
| `vc-wallet` | optional | optional | `capabilities.vc_wallet` (default `false`) |
| `issuer-signing` | no | yes | `capabilities.issuer_signing` (forced `false` for Verifier) |
| `KeyVault` | not loaded | two keys | — |
| writes | none | anchor writes + self-management + assertion signing | — |

`did-sdk<Server>` is constructed internally with `key_vault = None`: signing keys stay on the
`chain-sdk` side and pass through the namespace gate. The `Client`/`Server` choice is **not**
exposed in `chain-sdk` configuration.

---

## 3. Public Contract

### 3.1 `ChainSdk` trait

```rust
#[async_trait]
pub trait ChainSdk: SelectiveDisclosure + Send + Sync {
    fn participant_did(&self) -> &str;

    async fn health_check(&self) -> Result<HealthStatus, ChainSdkError>;

    async fn get_anchor(&self, anchor_id: &str) -> Result<AnchorRecord, ChainSdkError>;
    async fn list_anchors(
        &self,
        anchor_type: Option<AnchorType>,
        submitter_id: Option<u32>,
    ) -> Result<Vec<AnchorRecord>, ChainSdkError>;

    async fn get_participant(&self, participant_id: u32) -> Result<ParticipantRecord, ChainSdkError>;
    async fn get_participant_by_did(&self, did: &str) -> Result<Option<ParticipantRecord>, ChainSdkError>;
    async fn list_participants(&self) -> Result<Vec<ParticipantRecord>, ChainSdkError>;

    async fn resolve_did(&self, did: &str) -> Result<Option<DidRecord>, ChainSdkError>;
    fn resolve_did_key(&self, did: &str) -> Result<Ed25519PublicKey, ChainSdkError>;
    async fn resolve_did_web(&self, did: &str) -> Result<DidDocument, ChainSdkError>;

    async fn create_anchor(
        &self,
        anchor_type: AnchorType,
        payload_hash: &str,
        valid_until: u64,
    ) -> Result<String, ChainSdkError>;
    async fn revoke_anchor(&self, anchor_id: &str) -> Result<(), ChainSdkError>;
    async fn renew_anchor(&self, anchor_id: &str, new_valid_until: u64) -> Result<(), ChainSdkError>;

    async fn update_authorized_domains(
        &self,
        participant_id: u32,
        domains: Vec<String>,
    ) -> Result<(), ChainSdkError>;
    async fn update_controller_did(
        &self,
        participant_id: u32,
        new_controller_did: &str,
    ) -> Result<(), ChainSdkError>;

    async fn sign_participant_assertion(
        &self,
        payload: &[u8],
        purpose: AssertionPurpose,
    ) -> Result<Ed25519Signature, ChainSdkError>;

    fn subscribe_event(
        &self,
        event_name: &str,
        callback: Box<dyn Fn(EventData) + Send + Sync>,
    ) -> Result<Unsubscribe, ChainSdkError>;
}

pub type Unsubscribe = Box<dyn FnOnce() + Send + Sync>;
```

**Semantics**

- `participant_did` returns `participant.controller_did`; required for SD-JWT issuance.
- `valid_until == 0` means permanent. `renew_anchor` on a permanent anchor returns
  `AnchorPermanent`; `revoke_anchor` on a revoked anchor returns `AnchorRevoked`.
- `update_*` may only target the instance's own `participant_id`; otherwise `Unauthorized`.
- `subscribe_event` is synchronous. Implementations keep callbacks in `std::sync::RwLock` and
  never hold the lock across `.await`. `"*"` subscribes to all event names. Cancellation is
  idempotent via the returned `FnOnce`.
- `resolve_did` resolves the **on-chain** DID record from the backend; it is not delegated to
  `did-sdk`. `resolve_did_key` is offline `did:key` decoding; `resolve_did_web` delegates to
  `did-sdk<Server>` (HTTPS).

### 3.2 `SelectiveDisclosure` (supertrait)

```rust
#[async_trait]
pub trait SelectiveDisclosure: Send + Sync {
    async fn issue_sd_credential(
        &self,
        request: SdIssueRequest,
    ) -> Result<SdIssuedCredential, ChainSdkError>;

    async fn verify_sd_presentation(
        &self,
        presentation: &SdPresentation,
        expected: &SdExpected,
    ) -> Result<SdVerificationReport, ChainSdkError>;
}
```

`SdPresentation` is `did_sdk::VerifiablePresentation`; `SdDisclosure` is `did_sdk::Disclosure`.
See §7.

### 3.3 Data types

| Type | Fields (normative) |
|---|---|
| `ParticipantRecord` | `participant_id: u32`, `controller_did: String`, `operator: String` (SS58), `chain_role: ChainRole`, `business_types: Vec<BusinessType>`, `authorized_domains: Vec<String>`, `active: bool` |
| `AnchorRecord` | `anchor_id`, `anchor_type: AnchorType`, `submitter_id: u32`, `issuer_did: String`, `delegated_by_did: Option<String>`, `payload_hash: String`, `revoked: bool`, `created_at: u64`, `valid_until: u64` |
| `DidRecord` | `did: String`, `doc_hash: Option<String>`, `owner: Option<String>`, `block_number: Option<u64>` |
| `DidDocument` | `id: String`, `verification_methods: Vec<VerificationMethod>` |
| `VerificationMethod` | `id`, `controller`, `public_key_bytes: Vec<u8>` |
| `EventData` | `event_name: String`, `anchor_id: Option<String>`, `participant_id: Option<u32>`, `block_number: Option<u64>`, `payload: serde_json::Value` |
| `HealthStatus` | `healthy: bool`, `mode: String`, `detail: Option<String>`, `block_number: Option<u64>` |
| `Ed25519Signature` / `Sr25519Signature` | newtype over `Vec<u8>` |
| `Ed25519PublicKey` | newtype over `[u8; 32]` |

`AnchorType`: `EmailBinding`, `DomainControl`, `DidDocument`, `Warrant`, `Delegation`,
`OrgMembership`, `IssuedVc`, `GatewayAccess`, `Custom(String)`. JSON form:
`{"kind":"EmailBinding"}` or `{"kind":"Custom","name":"..."}`.

`BusinessType`: `ChainOwner`, `LegalAnchor`, `CarrierWitness`, `AppMembership`, `EmailBinding`,
`DomainControl`, `EmailGateway`, `RelyingParty`, `Warrant`, `Delegation`. **Never used for
authorization.**

`AssertionPurpose`: `ParticipantRegistration`, `VcIssuance`, `DelegationAck`.

### 3.4 Errors

`ChainSdkError` variants:

| Group | Variants |
|---|---|
| Anchor | `AnchorNotFound`, `AnchorTypeNotRegistered`, `AnchorPermanent`, `AnchorRevoked`, `AnchorExpired`, `AnchorPayloadMismatch` |
| Participant | `ParticipantNotFound`, `ParticipantDidNotFound`, `ParticipantInactive` |
| Access | `Unauthorized`, `KeyNamespaceViolation`, `SignerUnavailable` |
| Transport | `ChainUnavailable`, `NetworkError` |
| Input / signing | `InvalidArgument`, `SigningFailed`, `UserRejected`, `Unsupported`, `ConfigError` |
| Selective disclosure | `DisclosureHashMismatch`, `DisclosureDuplicate`, `DisclosureMissing`, `SdAlgUnsupported` |

`KeyVaultError` is mapped through **one** function (`map_vault_err`). All 13 variants are covered:

| `KeyVaultError` | `ChainSdkError` |
|---|---|
| `KeyNotFound` | `SignerUnavailable` |
| `KeyAlreadyExists` | `InvalidArgument` |
| `UserRejected` | **`UserRejected` (1:1, no string comparison)** |
| `InvalidPassword`, `Locked` | `SignerUnavailable` |
| `SigningFailed` | `SigningFailed` |
| `IoError` | `ChainUnavailable` |
| `InvalidKeyFormat`, `SerializationError` | `InvalidArgument` |
| `DecryptionFailed` | `SigningFailed` |
| `Unsupported` | `InvalidArgument` |
| `NamespaceViolation` | **`KeyNamespaceViolation`** |
| `AlgorithmMismatch` | `InvalidArgument` |

---

## 4. Keys and Role Gates

### 4.1 Two service-side key namespaces

| Namespace | Algorithm | Purpose | Only entry point |
|---|---|---|---|
| `chain-account/` | sr25519 | extrinsic signing, HTTP write-body signing | `keys::sign_extrinsic` |
| `participant-did/` | ed25519 | participant `did:web` assertion signing | `keys::sign_participant_assertion_inner` |

Both entry points validate the prefix **before** calling the vault and return
`KeyNamespaceViolation` on mismatch. `keyvault-core` independently validates namespace and
algorithm, giving two layers of protection. Return types differ (`Sr25519Signature` vs
`Ed25519Signature`), so the two paths cannot be mixed at the type level.

### 4.2 Key provisioning

`keyvault-core` generates `KeyId`s from public keys:
`chain-account/<SS58>` and `participant-did/did_key_<multibase>`. It has no import API and does
not accept caller-chosen local ids.

Therefore `key_vault.keys.operator` and `participant_did` must reference **existing** keys.
`chain-sdk` never silently creates identity keys. `mini_cli init-keys` generates both keys in the
configured `FileKeystore` and prints the actual `KeyId`s for configuration. `health_check` on an
`Issuer` reports missing keys.

### 4.3 Password resolution

Only for `Issuer` and `scheme = "file-keystore"`:

```text
explicit override
  → env(password_env, default CHAIN_KEYSTORE_PASSWORD)
  → password_file
  → allow_auto_generate_password (32 random bytes, file mode 0600)
  → ConfigError
```

`scheme = "memory-keystore"` needs no password and is intended for tests / Mock.

### 4.4 Role gate order (normative)

All writes and assertion signing MUST call the gate **before any backend logic or vault call**:

| Method | Gate |
|---|---|
| `create_anchor`, `revoke_anchor`, `renew_anchor` | `ensure_issuer()` |
| `update_authorized_domains`, `update_controller_did` | `ensure_self(participant_id)` (calls `ensure_issuer` then checks self) |
| `sign_participant_assertion` | `ensure_signer()` |

`business_types` is never read by the gate.

---

## 5. Backend Behaviour

### 5.1 `Mock`

- State: anchors, participants, `did_participant` index, `account_participant` index,
  registered anchor types, DID records, internal time and anchor sequence.
- Optional JSON persistence via `mock.state_path` (atomic write: temp file + `sync_all` + rename).
- Fault injection for tests: `Latency { op, millis }`, `Unavailable`, `Specific { op, error }`,
  plus `set_chain_unavailable(true)`.
- Events emitted: `AnchorCreated`, `AnchorRevoked`, `AnchorRenewed`, `ParticipantUpdated`.
- `created_at` uses the internal time line (`advance_time`).

### 5.2 `Centralized`

**HTTP reads**

| Operation | Request |
|---|---|
| health | `GET /health` → `{healthy, block_number?, detail?}` |
| anchor | `GET /anchors/{anchor_id}` (404 → `AnchorNotFound`) |
| anchors | `GET /anchors?anchor_type={kind}&submitter_id={id}` |
| participants | `GET /participants` |
| participant | `GET /participants/{participant_id}` |
| participant by DID | `GET /participants/by-did/{did}` (404 → `Ok(None)`) |
| DID | `GET /dids/{did}` (404 → `Ok(None)`) |

Path segments are percent-encoded with `reqwest::Url::path_segments_mut`, so DIDs containing `:`
and `/` are safe.

**HTTP writes (Issuer only)**

| Operation | Request |
|---|---|
| create anchor | `POST /anchors` ← `{anchor_type, payload_hash, valid_until}` → `{anchor_id}` |
| revoke | `POST /anchors/{id}/revoke` ← `{}` |
| renew | `POST /anchors/{id}/renew` ← `{valid_until}` |
| domains | `POST /participants/{id}/domains` ← `{domains}` |
| controller DID | `POST /participants/{id}/controller-did` ← `{controller_did}` |

Signature:

```text
payload = op || "\n" || canonical_json(body)
headers = x-chain-signer: <KeyId>
          x-chain-signature: <lowercase hex(sr25519 signature)>
          x-chain-op: <op>
```

`canonical_json`: recursive ascending object keys, compact separators, UTF-8; **floating point
numbers are rejected**. `op` values are the method names above.

**Cache.** Only `get_anchor` is cached (TTL = `cache_ttl_secs`; `0` disables). `revoke_anchor` and
`renew_anchor` invalidate the cached entry.

**Events.** `GET {endpoint}/events` WebSocket; each text frame is one `EventData` JSON object.
Connection is established lazily on the first subscription, reconnects with a 1 s delay, and stays
open for the process lifetime. Unsubscribing removes the callback only.

**Error mapping.** 401/403 → `Unauthorized`; 409 → `InvalidArgument` (revoke/renew → `AnchorRevoked`);
5xx → `ChainUnavailable`; other non-success → `NetworkError`; 404 handled per endpoint.

### 5.3 `Chain` (subxt POC)

- Connection: `OnlineClient::<SubstrateConfig>::from_url(chain_endpoint)` (runtime metadata).
- Implemented: connection, `health_check` (current block), finalized block **event stream**
  (`stream_blocks` → `at.events().fetch()`), role gates, DID resolution, assertion signing.
- **Not implemented in v0.1.0**: storage queries and extrinsic submission return
  `Unsupported`. Reasons:
  1. no identity-chain pallet/storage metadata is published in this repository;
  2. `subxt::tx::Signer::sign` is infallible (`-> T::Signature`), while `KeyVault::sign` is async
     and fallible (`UserRejected`), so errors cannot be propagated safely.
- Role gates run **before** the `Unsupported` return, so S9/S17/S19 hold in Chain mode.
- Real-chain integration tests are `#[ignore]`; see `metadata/README.md` for the metadata
  workflow.

### 5.4 Consistency

Mock and Centralized must return the same result or the same error variant for the same logical
operation. `tests/consistency.rs` covers reads, 404s, role violations and health. Chain POC reads
are intentionally `Unsupported` and excluded from value equality.

---

## 6. Participant Identity and Delegated Anchors

### 6.1 Three-part binding (S15)

```text
controller_did (did:web, identity root)
      ↔ participant_id (on-chain primary key)
      ↔ operator (SS58 chain account)
```

`did_participant` and `account_participant` indexes are maintained together.
`update_controller_did` migrates the DID index and rejects a DID already bound to another
participant. `operator` is never changed by `chain-sdk` (governance-only operation).

### 6.2 `business_types` (S20)

`business_types` is descriptive metadata for discovery/UX. It **must not** be consulted for
authorization. The only inputs are `SdkRole`, `chain_role` (informational) and
`authorized_domains`.

### 6.3 Delegated anchor semantics (S16)

| Field | Meaning |
|---|---|
| `issuer_did` | semantic issuer = **trust-chain root** (the delegating party) |
| `submitter_id` | actual submitter participant (governance audit) |
| `delegated_by_did` | proxy/delegate DID (transparent audit); `None` for self-submission |

`chain-sdk::create_anchor` always self-submits: `issuer_did = controller_did`,
`delegated_by_did = None`. Delegated anchors are submitted by
`chain-owner-sdk::submit_delegated_anchor`; `chain-sdk` only reads them.

---

## 7. VC Selective Disclosure (SD-JWT)

### 7.1 Model

Selective disclosure is added **without changing the on-chain anchor primitive**: the chain stores
one hash of the whole VC. Only the VC body, the disclosures and the SDK capabilities are extended.

| Artifact | Content |
|---|---|
| VC | `did_sdk::VerifiableCredential`; `credentialSubject._sd` = salted hashes, `_sd_alg = "sha-256"`; anchor id in `anchor_id` |
| Disclosures | `did_sdk::Disclosure { salt, name, value }`, delivered with the VC (not inside it) |
| VP | `did_sdk::VerifiablePresentation` with `disclosures: Some([...])` and holder proof |
| Anchor | `payload_hash` over the VC without `proof` and `anchor_id`; lifecycle unchanged |

The private key for the issuer proof is the `participant-did/` key, accessed only through
`sign_participant_assertion`.

### 7.2 Salted hash (`sd-v1`)

```text
salt        = base64url_nopad(16 random bytes)      # 16-byte CSPRNG salt per claim per VC
name_b64    = base64url_nopad(utf8(name))
value_b64   = base64url_nopad(utf8(value))          # strings as-is; other JSON as compact JSON
digest      = SHA-256(salt || name_b64 || value_b64)
_sd[i]      = base64url_nopad(digest)
```

This is the formula used by `did-sdk` / `keyvault-core` v0.2 (`Disclosure::hash_string`).
A fresh salt is generated for **each claim of each VC**, preventing cross-VC correlation and
dictionary attacks.

**Golden vector**

```text
salt_raw   = 000102030405060708090a0b0c0d0e0f
salt       = AAECAwQFBgcICQoLDA0ODw
name       = name            -> bmFtZQ
value      = Alice           -> QWxpY2U
input      = AAECAwQFBgcICQoLDA0ODwbmFtZQQWxpY2U
_sd[0]     = XGuolHxMRBLapsPiERv_dGyHZObbvA3efbPCxjsfbZA
```

### 7.3 Issuance (Issuer)

```text
1. caller selects mandatory claims (plaintext) and disclosable claims
2. for each disclosable claim: generate 16-byte salt, compute digest, add to _sd
3. build VC (id, type, issuer, issuanceDate, expirationDate?, credentialSubject)
4. payload_hash = "blake2b:" + BLAKE2b-256(compact VC without proof/anchor_id)
5. anchor_id = create_anchor(anchor_type, payload_hash, valid_until)
6. set VC.anchor_id
7. issuer proof = sign_participant_assertion(IssuedVc-v1 input)   # covers anchor_id
8. return { VC, disclosures, anchor_id, payload_hash }
```

Delivery of `{VC, disclosures}` to the holder is an application concern; `did-sdk` v0.2 stores
them together (`store_vc_with_disclosures`).

### 7.4 Presentation (`VP-SD-v1`)

The holder builds the VP with `did-sdk` v0.2 (`build_vp_selective`). The holder signature covers:

```text
VP-SD-v1
{holder}
{challenge}
{domain}
{disclosures_hash}
{vc_hash_1}
...
{vc_hash_n}
```

- `disclosures_hash` = hex(SHA-256(compact JSON array of disclosures));
- `vc_hash_i` = hex(BLAKE2b-256(compact full VC JSON, including proof)).

Unselected disclosures are not sent. `_sd` in the VC still contains their hashes.

### 7.5 Verification (Verifier)

```text
1. compare challenge and domain with the session
2. did-sdk verify_vp: holder proof, VC proofs, disclosure hashes, _sd_alg
3. for each VC: anchor_id exists
4. get_anchor(anchor_id): revoked? expired? (valid_until != 0 && now > valid_until)
5. recompute payload_hash and compare with the anchored value
6. return { verified, anchor_id, disclosed map, undisclosed_count }
```

Step 2 is reused from `did-sdk` v0.2 (`DidSdk::<Server>::verify_vp`), so chain-sdk and did-sdk
cannot drift on the holder/VC/disclosure rules. Anchor checks remain chain-sdk's responsibility
(R1).

### 7.6 SD error codes

| Code | Cause |
|---|---|
| `DisclosureHashMismatch` | recomputed digest is not in `_sd` (tampered salt/name/value) |
| `DisclosureDuplicate` | the same claim name is disclosed twice |
| `DisclosureMissing` | VP has no `disclosures`, VC has no `_sd`, or VC has no `anchor_id` |
| `SdAlgUnsupported` | `_sd_alg` is missing or not `sha-256` |
| `AnchorPayloadMismatch` | recomputed anchor hash differs (VC body tampered) |
| `AnchorRevoked` / `AnchorExpired` | anchor lifecycle failed |

### 7.7 Privacy boundary (honest disclosure)

| Threat | Mitigated | Mechanism |
|---|---|---|
| Verifier sees undisclosed values | yes | only selected disclosures are sent |
| Verifier forges/tampers disclosed values | yes | digest match + issuer proof |
| Verifier forges existence of an undisclosed claim | yes | its hash is in `_sd` |
| Cross-VC correlation (ID card vs. driver licence) | yes | independent random salt per VC per claim |
| Dictionary attack on a value | yes | random salt |
| Same-VC correlation across presentations | **no** | `_sd` hashes are fixed per VC |
| Multiple verifiers colluding to assemble a profile | **no** | information once disclosed cannot be revoked |
| Verifier storing disclosed values | **no** | out of cryptographic scope |

Mitigations: minimum-disclosure policy, verifier registration/audit, short VP lifetime, and
eventually BBS+ predicates.

### 7.8 BBS+ upgrade path

BBS+ is **not** in this release. It becomes relevant when many verifiers may collude, the same VC
is shown frequently, or predicate proofs (e.g. "age > 18") are required. The intended path is a
coexisting `_bbs` field with a format selector, not a replacement of `_sd`.

---

## 8. Configuration

### 8.1 TOML

```toml
[chain_sdk]
mode = "chain"                  # mock | centralized | chain
role = "issuer"                 # verifier | issuer (app is an alias)
participant_id = 4
cache_ttl_secs = 300
valid_until_refresh_window_secs = 86400
require_finalized_node = true

[chain_sdk.key_vault]
scheme = "file-keystore"        # file-keystore | memory-keystore
path = "./participant-keys"
password_env = "CHAIN_KEYSTORE_PASSWORD"
password_file = "./participant-keys/password"
allow_auto_generate_password = true

[chain_sdk.key_vault.keys]
operator = "chain-account/..."                 # actual KeyId from init-keys
participant_did = "participant-did/did_key_..." # actual KeyId from init-keys

[chain_sdk.participant]
controller_did = "did:web:mail.example.com:issuer"

[chain_sdk.did]
vc_storage_path = "./participant-vc-store"

[chain_sdk.mock]
# state_path = "./mock-state.json"              # optional persistence

[chain_sdk.capabilities]
did_resolution = true
vc_wallet = false
issuer_signing = true          # forced false for Verifier

[chain_sdk.centralized]
endpoint = "http://localhost:8080"

[chain_sdk.chain]
endpoint = "wss://rpc.identity-chain.com:9944"
```

### 8.2 Environment variables

| Variable | Overrides |
|---|---|
| `CHAIN_SDK_HOME` / `CHAIN_SDK_CONFIG_FILE` | home directory / config file path |
| `CHAIN_SDK_MODE` / `CHAIN_SDK_ROLE` | `mock|centralized|chain`, `verifier|issuer` |
| `CHAIN_SDK_PARTICIPANT_ID` / `CHAIN_SDK_CONTROLLER_DID` | identity |
| `CHAIN_KEYSTORE_PASSWORD` | keystore password (Issuer, file-keystore) |
| `CHAIN_SDK_KEYVAULT_SCHEME` / `_PATH` / `_PASSWORD_FILE` | keystore |
| `CHAIN_SDK_OPERATOR_KEY_ID` / `CHAIN_SDK_PARTICIPANT_DID_KEY_ID` | key ids |
| `CHAIN_SDK_VC_WALLET` / `CHAIN_SDK_ISSUER_SIGNING` | capabilities |
| `CHAIN_SDK_CENTRALIZED_ENDPOINT` / `CHAIN_SDK_CHAIN_ENDPOINT` | endpoints |
| `CHAIN_SDK_REQUIRE_FINALIZED` / `CHAIN_SDK_CACHE_TTL_SECS` | behaviour |
| `CHAIN_SDK_DID_VC_STORAGE_PATH` / `CHAIN_SDK_MOCK_STATE_PATH` | storage |

**Precedence:** explicit `ConfigOverrides` > environment > TOML > defaults.
Switching `mode` requires a process restart.

**Requirement:** `participant.controller_did` must be non-empty when building an SDK (both roles),
because `did-sdk<Server>` requires a participant DID. `resolve_config` itself allows an empty
value so that configuration can be inspected without building.

---

## 9. Platform and Build

### 9.1 Targets

| Target | Runner | Requirement |
|---|---|---|
| `x86_64-unknown-linux-musl` | ubuntu-latest | fully static (`file`/`ldd` assertion) |
| `aarch64-unknown-linux-musl` | ubuntu-latest | fully static |
| `aarch64-apple-darwin` | macos-15 | native |
| `x86_64-pc-windows-msvc` | windows-latest | native |

Cross-compilation is performed **only on GitHub Actions runners**. Toolchain: Rust 1.88.0
(`edition 2024`, `resolver = "3"`). Linux musl uses `cargo-zigbuild` + zig 0.13.

### 9.2 Dependencies and static linking

- TLS: `reqwest(rustls-tls)` and `tokio-tungstenite(rustls-tls-webpki-roots)`; `ring` is accepted
  as the rustls provider (pure Rust + assembly, no system library).
- Forbidden: `native-tls`, `openssl`, non-bundled `-sys` dependencies.
- `subxt` exists only in `chain-sdk`; `did-sdk`'s dependency tree must not contain it (R1).
- `keyvault-core` and `did-sdk` are external git dependencies pinned to tag `v0.2.0`.
- `Cargo.lock` is committed and CI runs with `--locked`.

### 9.3 CI

| Workflow | Checks |
|---|---|
| `ci` | `cargo fmt --check`, clippy `-D warnings`, `cargo test --all-features`, trait method completeness |
| `rust-musl` | four-target build matrix, static assertions, artifact upload |
| `consistency` | Mock vs Centralized semantics + role gate |
| `roles` | role matrix, S18/S17 grep assertions, `did-sdk` has no `subxt` |

---

## 10. Security Considerations

- Private keys never leave `keyvault-core`; `SecretString` is redacted in `Debug`.
- Namespace and algorithm checks occur in both `chain-sdk` and `keyvault-core`.
- Role checks precede all backend and vault operations.
- Centralized writes sign `op + canonical JSON`; the canonical form rejects floats and is
  documented so the service can reproduce it byte-for-byte.
- `UserRejected` is propagated 1:1 from the vault, never string-matched.
- SD-JWT verification reuses `did-sdk` v0.2 for holder/VC/disclosure checks; anchor state is
  checked by `chain-sdk`.
- Mock mode is for tests only; `health_check.mode` exposes the active backend.
- The privacy limits in §7.7 are part of the contract and must not be overstated.

---

## 11. Acceptance Criteria

### 11.1 Core contract (S1–S20)

| ID | Criterion | Evidence |
|---|---|---|
| S1 | Complete trait: 17 methods | `cargo check`; CI method-list check |
| S2 | Three backends selectable by `mode` | `tests/config_resolution.rs`, `tests/role_matrix.rs` |
| S3 | Three-backend semantic consistency | `tests/consistency.rs` (Mock vs Centralized; Chain POC reads `Unsupported`) |
| S4 | Mock preload + persistence | `tests/mock_persistence.rs`, `tests/role_gate.rs` |
| S5 | Mock fault injection | `tests/mock_fault_injection.rs` |
| S6 | Centralized endpoint mapping | `tests/centralized_contract.rs` |
| S7 | Centralized write signature | `tests/centralized_signing.rs` |
| S8 | Centralized TTL cache | `tests/centralized_cache.rs` |
| S9 | Role interception in all backends | `tests/role_gate.rs`, `tests/role_matrix.rs` |
| S10 | Built-in key management | `tests/keys_namespace.rs`, config tests |
| S11 | Event subscription in all backends | `tests/event_subscription.rs` |
| S12 | Consistency suite | `tests/consistency.rs` |
| S13 | `keyvault-core` integration and error mapping | `tests/keyvault_integration.rs` |
| S14 | Namespace isolation | `tests/keys_namespace.rs` |
| S15 | Participant three-part binding | `tests/participant_binding.rs` |
| S16 | Delegated anchor semantics | `tests/participant_binding.rs` |
| S17 | Role-based capability assembly | `tests/capabilities.rs`, `src/config.rs` unit tests |
| S18 | No governance methods | `tests/no_governance.rs`, `scripts/verify-roles.sh` |
| S19 | Self-management scope | `tests/role_gate.rs` |
| S20 | `business_types` not used for authorization | `tests/participant_binding.rs` |

Additional platform criteria:

| ID | Criterion |
|---|---|
| P1 | `x86_64-unknown-linux-musl` fully static |
| P2 | `aarch64-unknown-linux-musl` fully static |
| P3 | `aarch64-apple-darwin` native build |
| P4 | `x86_64-pc-windows-msvc` native build |
| P5 | One `mini_cli` artifact per target |

### 11.2 Selective disclosure

| ID | Criterion | Evidence |
|---|---|---|
| I1 | 16-byte random salt per claim; `sd-v1` digest | `tests/sd_issue.rs` |
| I2 | Correct `_sd` and `_sd_alg` | `tests/sd_issue.rs` |
| I3 | Independent salts across claims and VCs | `tests/sd_issue.rs` |
| I4 | Disclosure format `[salt, name, value]` | `tests/sd_issue.rs` |
| I5 | Anchor hash unchanged (VC with `_sd`, without disclosures) | `tests/sd_issue.rs` |
| I6 | VC + disclosures delivered together | `did-sdk` v0.2 (`store_vc_with_disclosures`) |
| V1 | Holder signature and challenge | `tests/sd_verify.rs` |
| V2 | Issuer VC signature | `tests/sd_verify.rs` |
| V3 | Disclosure digest match | `tests/sd_verify.rs` |
| V4 | Undisclosed values invisible | `tests/sd_verify.rs` |
| V5 | Anchor revoked/expired check | `tests/sd_verify.rs` |
| V6 | Tamper detection | `tests/sd_verify.rs` |
| V7 | Duplicate disclosure detection | `tests/sd_verify.rs` |
| P1 | Cross-VC unlinkability | `tests/sd_issue.rs` |
| P2 | Same-VC linking documented as accepted | §7.7 |
| P3 | Dictionary-attack protection | random salt |

Holder-side storage, selection UI and VP construction (D1–D5) are `did-sdk` responsibilities and
are validated in that repository.

---

## 12. Implementation Status and Known Limits (v0.1.0)

- Local: 96 tests pass; `cargo fmt --check` and `cargo clippy -D warnings` clean.
- CI: four workflows green on the first `main` push (ci, rust-musl, consistency, roles).
- `Chain` backend: connection, health, events and role gates implemented; storage/extrinsic
  operations return `Unsupported` (see §5.3).
- Centralized service contract is the one defined in §5.2; it must be confirmed with the
  centralized-chain implementation (open item).
- Canonical JSON is deterministic for this implementation; RFC 8785 (JCS) is not yet adopted.
- `sd-jwt` Cargo feature is reserved; the implementation is always compiled.

---

## 13. Change Control

1. This document is the single normative source; code comments refer to it.
2. Breaking changes to `ChainSdk`, `ChainSdkError`, configuration fields or the centralized
   contract require a document update first, then code, then a release note.
3. Acceptance evidence lives in the test suite and CI; the tables in §11 must be kept in sync
   when tests move.
4. The document filename is intentionally unversioned; the package version in §12 identifies the
   release.
