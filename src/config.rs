//! 配置模型、默认值、覆盖项与 SDK 工厂入口。
//!
//! 优先级（P0 实现）：显式 [`ConfigOverrides`] > 环境变量 > TOML 文件 > 内置默认值。
//! 密钥库密码单独一条链（仅 `role = Issuer` 时解析）：
//! `CHAIN_KEYSTORE_PASSWORD` → `password_file` → `allow_auto_generate_password`
//! 自动生成 → 报错。
//!
//! TOML 各段与结构体的对应关系见 `docs/implementation-plan.md` 附录 B。

use std::path::PathBuf;
use std::sync::Arc;

use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use crate::{ChainSdk, ChainSdkError};

/// 后端模式。切换需要重启进程。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChainSdkMode {
    /// 进程内 Mock（无外部依赖）。
    #[default]
    Mock,
    /// 中心化模拟链（HTTP）。
    Centralized,
    /// 真实 Substrate 链（subxt）。
    Chain,
}

impl ChainSdkMode {
    /// 稳定的小写标识（用于日志与 [`crate::HealthStatus::mode`]）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mock => "mock",
            Self::Centralized => "centralized",
            Self::Chain => "chain",
        }
    }
}

/// SDK 构造角色。
///
/// **两角色线性包含**：`Verifier ⊂ Issuer`。`role = "app"` 是 `Issuer` 的
/// 反序列化别名，向后兼容旧配置；不存在独立的 `App` 变体。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SdkRole {
    /// 只读：不加载 `KeyVault`，无写能力与断言签名。
    #[default]
    Verifier,
    /// 读写：加载 `chain-account/` 与 `participant-did/` 两类密钥。
    #[serde(alias = "app")]
    Issuer,
}

impl SdkRole {
    /// 是否需要加载 `KeyVault`（仅 Issuer）。
    pub fn needs_keyvault(self) -> bool {
        matches!(self, Self::Issuer)
    }

    /// 该角色是否加载 `issuer-signing` 能力（仅 Issuer）。
    pub fn has_issuer_signing(self) -> bool {
        self.needs_keyvault()
    }
}

/// 两类服务端密钥的 `KeyId`。
///
/// 前缀由实现强制校验（S14）：
/// - `operator` 必须以 `chain-account/` 开头（sr25519）；
/// - `participant_did` 必须以 `participant-did/` 开头（ed25519）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyVaultKeys {
    /// 链账户密钥（sr25519），如 `chain-account/operator-1`。
    pub operator: String,
    /// 参与方 did:web 密钥（ed25519），如 `participant-did/did_web_example_com`。
    pub participant_did: String,
}

impl Default for KeyVaultKeys {
    fn default() -> Self {
        Self {
            operator: "chain-account/operator-1".into(),
            participant_did: "participant-did/did_web_example_com".into(),
        }
    }
}

/// 密钥库配置。
///
/// `scheme` 当前支持 `file-keystore` 与 `memory-keystore`；未来
/// `keyvault-device` / `keyvault-server` 通过新增 scheme + feature 门控接入。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct KeyVaultConfig {
    /// 密钥库 scheme。
    pub scheme: String,
    /// 密钥库目录（`file-keystore` 使用）。
    pub path: PathBuf,
    /// 密码所在环境变量名。
    pub password_env: String,
    /// 密码文件路径（不存在且允许自动生成时写入）。
    pub password_file: PathBuf,
    /// 是否允许自动生成密码。
    pub allow_auto_generate_password: bool,
    /// 两类密钥的 `KeyId`。
    pub keys: KeyVaultKeys,
}

impl Default for KeyVaultConfig {
    fn default() -> Self {
        let home = home_dir();
        Self {
            scheme: "file-keystore".into(),
            path: home.join("participant-keys"),
            password_env: "CHAIN_KEYSTORE_PASSWORD".into(),
            password_file: home.join("participant-keyvault.password"),
            allow_auto_generate_password: true,
            keys: KeyVaultKeys::default(),
        }
    }
}

/// 参与方自身配置。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ParticipantConfig {
    /// 本参与方的 `did:web`（必须与链上 `ParticipantRecord.controller_did` 一致）。
    pub controller_did: String,
}

/// `did-sdk<Server>` 装配参数。
///
/// VC 存储与 Disclosures 存储均由 `did-sdk` 管理（S17）；`chain-sdk` 只提供
/// 目录与密码来源，不读取、不写入 VC 文件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DidConfig {
    /// VC 存储目录。
    pub vc_storage_path: PathBuf,
}

impl Default for DidConfig {
    fn default() -> Self {
        Self {
            vc_storage_path: home_dir().join("participant-vc-store"),
        }
    }
}

/// Mock 后端配置。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MockConfig {
    /// 状态持久化路径；`None` 表示不落盘（纯内存）。
    pub state_path: Option<PathBuf>,
}

/// 按角色装配的能力开关（S17）。
///
/// - `did_resolution`：两角色都需要（默认开启）；
/// - `vc_wallet`：可选，POC 默认关闭；
/// - `issuer_signing`：仅 Issuer；Verifier 构造时被强制为 `false`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Capabilities {
    /// 是否启用 DID 解析。
    pub did_resolution: bool,
    /// 是否启用 VC 钱包。
    pub vc_wallet: bool,
    /// 是否启用签发方签名。
    pub issuer_signing: bool,
}

impl Default for Capabilities {
    fn default() -> Self {
        Self {
            did_resolution: true,
            vc_wallet: false,
            issuer_signing: false,
        }
    }
}

impl Capabilities {
    /// 返回按角色收敛后的有效能力：Verifier 的 `issuer_signing` 恒为 `false`。
    pub fn effective_for(self, role: SdkRole) -> Self {
        Self {
            did_resolution: self.did_resolution,
            vc_wallet: self.vc_wallet,
            issuer_signing: self.issuer_signing && role.has_issuer_signing(),
        }
    }
}

/// `chain-sdk` 根配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ChainSdkConfig {
    /// 后端模式。
    pub mode: ChainSdkMode,
    /// SDK 构造角色。
    pub role: SdkRole,
    /// 本参与方 `participant_id`（自身管理写的范围）。
    pub participant_id: u32,
    /// 密钥库（仅 Issuer 加载）。
    pub key_vault: KeyVaultConfig,
    /// 参与方自身配置。
    pub participant: ParticipantConfig,
    /// `did-sdk<Server>` 装配参数。
    pub did: DidConfig,
    /// Mock 后端参数。
    pub mock: MockConfig,
    /// 能力开关。
    pub capabilities: Capabilities,
    /// 中心化模拟链端点。
    pub centralized_endpoint: String,
    /// Substrate RPC 端点。
    pub chain_endpoint: String,
    /// 是否要求 finalized 节点。
    pub require_finalized_node: bool,
    /// `get_anchor` 的 TTL 缓存秒数；`0` 关闭缓存。
    pub cache_ttl_secs: u64,
    /// 续期提醒窗口（秒）。
    pub valid_until_refresh_window_secs: u64,
    /// 已解析的密钥库密码（不序列化、不打印）。
    #[serde(skip)]
    pub resolved_password: Option<SecretString>,
}

impl Default for ChainSdkConfig {
    fn default() -> Self {
        Self {
            mode: ChainSdkMode::default(),
            role: SdkRole::default(),
            participant_id: 0,
            key_vault: KeyVaultConfig::default(),
            participant: ParticipantConfig::default(),
            did: DidConfig::default(),
            mock: MockConfig::default(),
            capabilities: Capabilities::default(),
            centralized_endpoint: "http://127.0.0.1:8080".into(),
            chain_endpoint: "ws://127.0.0.1:9944".into(),
            require_finalized_node: true,
            cache_ttl_secs: 300,
            valid_until_refresh_window_secs: 86_400,
            resolved_password: None,
        }
    }
}

/// 显式覆盖项：优先级最高，通常来自 CLI 参数或调用方代码。
#[derive(Debug, Default, Clone, Deserialize)]
pub struct ConfigOverrides {
    /// `mock` / `centralized` / `chain`。
    pub mode: Option<String>,
    /// `verifier` / `issuer`（`app` 为别名）。
    pub role: Option<String>,
    /// 本参与方 id。
    pub participant_id: Option<u32>,
    /// 本参与方 `did:web`。
    pub controller_did: Option<String>,
    /// 密钥库 scheme。
    pub key_vault_scheme: Option<String>,
    /// 密钥库目录。
    pub key_vault_path: Option<PathBuf>,
    /// 显式密码（最高优先级）。
    pub key_vault_password: Option<String>,
    /// 密码文件。
    pub key_vault_password_file: Option<PathBuf>,
    /// 链账户密钥 `KeyId`。
    pub operator_key_id: Option<String>,
    /// 参与方 DID 密钥 `KeyId`。
    pub participant_did_key_id: Option<String>,
    /// 是否启用 VC 钱包。
    pub vc_wallet: Option<bool>,
    /// 是否启用签发方签名。
    pub issuer_signing: Option<bool>,
    /// 中心化端点。
    pub centralized_endpoint: Option<String>,
    /// 链端点。
    pub chain_endpoint: Option<String>,
    /// 是否要求 finalized。
    pub require_finalized_node: Option<bool>,
    /// 缓存 TTL。
    pub cache_ttl_secs: Option<u64>,
    /// VC 存储目录。
    pub did_vc_storage_path: Option<PathBuf>,
    /// Mock 状态持久化路径。
    pub mock_state_path: Option<PathBuf>,
}

/// 本机配置根目录。
///
/// 优先 `CHAIN_SDK_HOME`；否则 `dirs::data_dir()/chain-sdk`；再退化到当前目录。
pub fn home_dir() -> PathBuf {
    if let Ok(value) = std::env::var("CHAIN_SDK_HOME") {
        if !value.is_empty() {
            return PathBuf::from(value);
        }
    }
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("chain-sdk")
}

/// 解析配置（P0 实现；当前为初始化占位）。
///
/// 完整算法见 `docs/implementation-plan.md` 附录 B。
pub fn resolve_config(overrides: ConfigOverrides) -> Result<ChainSdkConfig, ChainSdkError> {
    let _ = overrides;
    Err(ChainSdkError::ConfigError(
        "resolve_config 尚未实现：初始化骨架，按实施文档 P0 落地".into(),
    ))
}

/// 按配置构造 `Arc<dyn ChainSdk>`（P0 实现；当前为初始化占位）。
///
/// Issuer 会加载 `KeyVault` 与两类密钥；Verifier 不加载 `KeyVault`（S17）。
pub fn build_chain_sdk(config: &ChainSdkConfig) -> Result<Arc<dyn ChainSdk>, ChainSdkError> {
    let _ = config;
    Err(ChainSdkError::Unsupported(
        "build_chain_sdk 尚未实现：初始化骨架，按实施文档 P0 落地".into(),
    ))
}
