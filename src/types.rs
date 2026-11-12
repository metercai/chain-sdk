//! 链上数据模型：参与方、锚定、DID、签名与健康状态。
//!
//! 字段命名与 `docs/specs/01-ChainSdk接口契约与实现规格书-v2.md` 完全一致；
//! JSON 序列化使用 snake_case 字段名（与中心化模拟链服务契约一致）。

use serde::{Deserialize, Serialize};

/// 参与方记录：`controller_did` ↔ `participant_id` ↔ `operator` 三段式硬绑定（S15）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParticipantRecord {
    /// 链上主键，单调递增。
    pub participant_id: u32,
    /// 参与方 `did:web`（身份根）。
    pub controller_did: String,
    /// 链账户 SS58 地址。
    pub operator: String,
    /// 链上角色（由链主治理分配）。
    pub chain_role: ChainRole,
    /// 业务类型：**语义描述，不参与鉴权**（S20）。
    pub business_types: Vec<BusinessType>,
    /// 授权域名；仅 Issuer 有意义。
    pub authorized_domains: Vec<String>,
    /// 是否激活。
    pub active: bool,
}

/// 链上角色（`ParticipantRecord.chain_role`）。
///
/// 与 SDK 构造角色 [`crate::SdkRole`] 是两个概念：本枚举由链主治理分配，
/// 决定**链上**能力边界；`SdkRole` 决定本进程加载哪些密钥与能力。
///
/// 线性包含：`Verifier ⊂ Issuer ⊂ ChainOwner`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ChainRole {
    /// 验证方。
    Verifier,
    /// 发行方。
    Issuer,
    /// 链主（治理）。
    ChainOwner,
}

/// 业务类型：仅用于语义描述与展示，**不得参与鉴权**（S20）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BusinessType {
    /// 链主。
    ChainOwner,
    /// 法律锚定。
    LegalAnchor,
    /// 承运见证。
    CarrierWitness,
    /// 应用成员。
    AppMembership,
    /// 邮箱绑定。
    EmailBinding,
    /// 域名控制。
    DomainControl,
    /// 邮件网关。
    EmailGateway,
    /// 依赖方。
    RelyingParty,
    /// 授权书。
    Warrant,
    /// 委托。
    Delegation,
}

/// 锚定记录。
///
/// - `issuer_did`：语义签发者 = **信任链根**（S16）；委托锚定时仍是被代理方；
/// - `submitter_id`：实际提交者（治理审计）；
/// - `delegated_by_did`：委托锚定的代理方（透明审计），自身提交时为 `None`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnchorRecord {
    /// 锚定 id。
    pub anchor_id: String,
    /// 锚定类型。
    pub anchor_type: AnchorType,
    /// 提交者 `participant_id`。
    pub submitter_id: u32,
    /// 语义签发者 DID（信任根）。
    pub issuer_did: String,
    /// 委托锚定的代理方 DID。
    pub delegated_by_did: Option<String>,
    /// 被锚定载荷的哈希（含算法前缀，如 `blake2b:...`）。
    pub payload_hash: String,
    /// 是否已撤销。
    pub revoked: bool,
    /// 创建时间（Unix 秒）。
    pub created_at: u64,
    /// 有效期（Unix 秒）；`0` 表示永久。
    pub valid_until: u64,
}

/// 链上 DID 记录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DidRecord {
    /// DID 字符串。
    pub did: String,
    /// DID 文档哈希（链上可选）。
    pub doc_hash: Option<String>,
    /// 拥有者 DID / 账户。
    pub owner: Option<String>,
    /// 所在区块号（若适用）。
    pub block_number: Option<u64>,
}

/// W3C DID Document 的扁平子集。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DidDocument {
    /// 文档 id。
    pub id: String,
    /// 验证方法列表。
    pub verification_methods: Vec<VerificationMethod>,
}

/// DID 文档中的验证方法。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationMethod {
    /// 验证方法 id。
    pub id: String,
    /// 控制者 DID。
    pub controller: String,
    /// Ed25519 公钥原始字节。
    pub public_key_bytes: Vec<u8>,
}

/// 锚定类型。
///
/// JSON 表示采用相邻标记：`{"kind":"EmailBinding"}` 或
/// `{"kind":"Custom","name":"..."}`。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "name")]
pub enum AnchorType {
    /// 邮箱绑定。
    EmailBinding,
    /// 域名控制。
    DomainControl,
    /// DID 文档。
    DidDocument,
    /// 授权书。
    Warrant,
    /// 委托。
    Delegation,
    /// 组织成员。
    OrgMembership,
    /// 已签发 VC。
    IssuedVc,
    /// 网关访问。
    GatewayAccess,
    /// 自定义类型（需链主 `register_anchor_type`）。
    Custom(String),
}

impl AnchorType {
    /// 是否为自定义类型。
    pub fn is_custom(&self) -> bool {
        matches!(self, Self::Custom(_))
    }

    /// 稳定的字符串表示（用于 HTTP 查询参数与日志）。
    ///
    /// 已知变体返回变体名；`Custom(name)` 返回内部名称。
    pub fn as_str(&self) -> &str {
        match self {
            Self::EmailBinding => "EmailBinding",
            Self::DomainControl => "DomainControl",
            Self::DidDocument => "DidDocument",
            Self::Warrant => "Warrant",
            Self::Delegation => "Delegation",
            Self::OrgMembership => "OrgMembership",
            Self::IssuedVc => "IssuedVc",
            Self::GatewayAccess => "GatewayAccess",
            Self::Custom(name) => name,
        }
    }
}

/// 参加方断言签名的用途（域分离标签）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AssertionPurpose {
    /// 参与方注册。
    ParticipantRegistration,
    /// VC 签发。
    VcIssuance,
    /// 委托确认。
    DelegationAck,
}

/// Ed25519 签名（`participant-did/` namespace）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Ed25519Signature(pub Vec<u8>);

impl Ed25519Signature {
    /// 原始签名字节。
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// Ed25519 公钥（32 字节）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Ed25519PublicKey(pub [u8; 32]);

impl Ed25519PublicKey {
    /// 原始公钥字节。
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// sr25519 签名（`chain-account/` namespace）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Sr25519Signature(pub Vec<u8>);

impl Sr25519Signature {
    /// 原始签名字节。
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// 后端健康状态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthStatus {
    /// 是否健康。
    pub healthy: bool,
    /// 后端模式（`mock` / `centralized` / `chain`）。
    pub mode: String,
    /// 诊断信息。
    pub detail: Option<String>,
    /// 区块高度（若适用）。
    pub block_number: Option<u64>,
}
