//! 配置模型、解析与 SDK 工厂。
//!
//! 优先级：显式 [`ConfigOverrides`] > 环境变量 > TOML 文件 > 内置默认值。
//! 密码单独一条链（仅 `role = Issuer` 且 `scheme = "file-keystore"` 时解析）：
//! `password_env` → `password_file` → `allow_auto_generate_password` → 报错。
//!
//! TOML 使用嵌套段（`[chain_sdk.key_vault]`、`[chain_sdk.centralized]` 等），
//! 由内部 `FileConfig` 映射到扁平的 [`ChainSdkConfig`]。完整算法见
//! `docs/ChainSdk-Interface-Contract.md` §8。

use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use keyvault_core::{FileKeystore, KeyVault, MemoryKeystore, PasswordSource, resolve_password};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use crate::error::map_vault_err;
use crate::{ChainSdk, ChainSdkError, SdkCore};

/// 后端模式。切换需要重启进程。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChainSdkMode {
    /// 进程内 Mock（无外部依赖）。
    #[default]
    Mock,
    /// 中心化模拟链（HTTP + WebSocket）。
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

    fn parse(value: &str) -> Result<Self, ChainSdkError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "mock" => Ok(Self::Mock),
            "centralized" => Ok(Self::Centralized),
            "chain" => Ok(Self::Chain),
            other => Err(ChainSdkError::ConfigError(format!(
                "mode expects mock|centralized|chain, got {other}"
            ))),
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

    fn parse(value: &str) -> Result<Self, ChainSdkError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "verifier" => Ok(Self::Verifier),
            "issuer" | "app" => Ok(Self::Issuer),
            other => Err(ChainSdkError::ConfigError(format!(
                "role expects verifier|issuer|app, got {other}"
            ))),
        }
    }
}

/// 两类服务端密钥的 `KeyId`。
///
/// 前缀由实现强制校验（S14）：
/// - `operator` 必须以 `chain-account/` 开头（sr25519）；
/// - `participant_did` 必须以 `participant-did/` 开头（ed25519）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyVaultKeys {
    /// 链账户密钥（sr25519），例如 `chain-account/5GrwvaEF...`。
    pub operator: String,
    /// 参与方 did:web 密钥（ed25519），例如 `participant-did/did_key_z6Mk...`。
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
/// 目录，不读取、不写入 VC 文件。
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

// ===== TOML 文件模型（全 Option，便于部分覆盖） =====

#[derive(Debug, Default, Deserialize)]
struct FileConfig {
    chain_sdk: Option<FileSection>,
}

#[derive(Debug, Default, Deserialize)]
struct FileSection {
    mode: Option<String>,
    role: Option<String>,
    participant_id: Option<u32>,
    cache_ttl_secs: Option<u64>,
    valid_until_refresh_window_secs: Option<u64>,
    require_finalized_node: Option<bool>,
    key_vault: Option<FileKeyVault>,
    participant: Option<FileParticipant>,
    did: Option<FileDid>,
    mock: Option<FileMock>,
    capabilities: Option<FileCapabilities>,
    centralized: Option<FileEndpoint>,
    chain: Option<FileChainSection>,
}

#[derive(Debug, Default, Deserialize)]
struct FileKeyVault {
    scheme: Option<String>,
    path: Option<PathBuf>,
    password_env: Option<String>,
    password_file: Option<PathBuf>,
    allow_auto_generate_password: Option<bool>,
    keys: Option<FileKeys>,
}

#[derive(Debug, Default, Deserialize)]
struct FileKeys {
    operator: Option<String>,
    participant_did: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct FileParticipant {
    controller_did: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct FileDid {
    vc_storage_path: Option<PathBuf>,
}

#[derive(Debug, Default, Deserialize)]
struct FileMock {
    state_path: Option<PathBuf>,
}

#[derive(Debug, Default, Deserialize)]
struct FileCapabilities {
    did_resolution: Option<bool>,
    vc_wallet: Option<bool>,
    issuer_signing: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
struct FileEndpoint {
    endpoint: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct FileChainSection {
    endpoint: Option<String>,
    require_finalized_node: Option<bool>,
}

fn apply_file(config: &mut ChainSdkConfig) -> Result<(), ChainSdkError> {
    let explicit = std::env::var("CHAIN_SDK_CONFIG_FILE")
        .ok()
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let path = explicit
        .clone()
        .unwrap_or_else(|| home_dir().join("chain-sdk.toml"));
    if !path.exists() {
        if explicit.is_some() {
            return Err(ChainSdkError::ConfigError(format!(
                "config file not found: {}",
                path.display()
            )));
        }
        return Ok(());
    }
    let raw = std::fs::read_to_string(&path)
        .map_err(|error| ChainSdkError::ConfigError(format!("read {}: {error}", path.display())))?;
    let file: FileConfig = toml::from_str(&raw).map_err(|error| {
        ChainSdkError::ConfigError(format!("parse {}: {error}", path.display()))
    })?;
    let Some(section) = file.chain_sdk else {
        return Ok(());
    };
    if let Some(mode) = section.mode {
        config.mode = ChainSdkMode::parse(&mode)?;
    }
    if let Some(role) = section.role {
        config.role = SdkRole::parse(&role)?;
    }
    if let Some(value) = section.participant_id {
        config.participant_id = value;
    }
    if let Some(value) = section.cache_ttl_secs {
        config.cache_ttl_secs = value;
    }
    if let Some(value) = section.valid_until_refresh_window_secs {
        config.valid_until_refresh_window_secs = value;
    }
    if let Some(value) = section.require_finalized_node {
        config.require_finalized_node = value;
    }
    if let Some(kv) = section.key_vault {
        if let Some(value) = kv.scheme {
            config.key_vault.scheme = value;
        }
        if let Some(value) = kv.path {
            config.key_vault.path = value;
        }
        if let Some(value) = kv.password_env {
            config.key_vault.password_env = value;
        }
        if let Some(value) = kv.password_file {
            config.key_vault.password_file = value;
        }
        if let Some(value) = kv.allow_auto_generate_password {
            config.key_vault.allow_auto_generate_password = value;
        }
        if let Some(keys) = kv.keys {
            if let Some(value) = keys.operator {
                config.key_vault.keys.operator = value;
            }
            if let Some(value) = keys.participant_did {
                config.key_vault.keys.participant_did = value;
            }
        }
    }
    if let Some(participant) = section.participant {
        if let Some(value) = participant.controller_did {
            config.participant.controller_did = value;
        }
    }
    if let Some(did) = section.did {
        if let Some(value) = did.vc_storage_path {
            config.did.vc_storage_path = value;
        }
    }
    if let Some(mock) = section.mock {
        if let Some(value) = mock.state_path {
            config.mock.state_path = Some(value);
        }
    }
    if let Some(capabilities) = section.capabilities {
        if let Some(value) = capabilities.did_resolution {
            config.capabilities.did_resolution = value;
        }
        if let Some(value) = capabilities.vc_wallet {
            config.capabilities.vc_wallet = value;
        }
        if let Some(value) = capabilities.issuer_signing {
            config.capabilities.issuer_signing = value;
        }
    }
    if let Some(endpoint) = section.centralized.and_then(|value| value.endpoint) {
        config.centralized_endpoint = endpoint;
    }
    if let Some(chain) = section.chain {
        if let Some(value) = chain.endpoint {
            config.chain_endpoint = value;
        }
        if let Some(value) = chain.require_finalized_node {
            config.require_finalized_node = value;
        }
    }
    Ok(())
}

fn env_string(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

fn env_parse<T>(name: &str) -> Result<Option<T>, ChainSdkError>
where
    T: FromStr,
    T::Err: std::fmt::Display,
{
    match env_string(name) {
        Some(value) => value.parse::<T>().map(Some).map_err(|error| {
            ChainSdkError::ConfigError(format!("{name} expects a valid value: {error}"))
        }),
        None => Ok(None),
    }
}

fn env_bool(name: &str) -> Result<Option<bool>, ChainSdkError> {
    let Some(value) = env_string(name) else {
        return Ok(None);
    };
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(Some(true)),
        "0" | "false" | "no" | "off" => Ok(Some(false)),
        other => Err(ChainSdkError::ConfigError(format!(
            "{name} expects bool, got {other}"
        ))),
    }
}

fn apply_env(config: &mut ChainSdkConfig) -> Result<(), ChainSdkError> {
    if let Some(value) = env_string("CHAIN_SDK_MODE") {
        config.mode = ChainSdkMode::parse(&value)?;
    }
    if let Some(value) = env_string("CHAIN_SDK_ROLE") {
        config.role = SdkRole::parse(&value)?;
    }
    if let Some(value) = env_parse::<u32>("CHAIN_SDK_PARTICIPANT_ID")? {
        config.participant_id = value;
    }
    if let Some(value) = env_string("CHAIN_SDK_CONTROLLER_DID") {
        config.participant.controller_did = value;
    }
    if let Some(value) = env_string("CHAIN_SDK_KEYVAULT_SCHEME") {
        config.key_vault.scheme = value;
    }
    if let Some(value) = env_string("CHAIN_SDK_KEYVAULT_PATH") {
        config.key_vault.path = PathBuf::from(value);
    }
    if let Some(value) = env_string("CHAIN_SDK_KEYVAULT_PASSWORD_FILE") {
        config.key_vault.password_file = PathBuf::from(value);
    }
    if let Some(value) = env_string("CHAIN_SDK_OPERATOR_KEY_ID") {
        config.key_vault.keys.operator = value;
    }
    if let Some(value) = env_string("CHAIN_SDK_PARTICIPANT_DID_KEY_ID") {
        config.key_vault.keys.participant_did = value;
    }
    if let Some(value) = env_bool("CHAIN_SDK_VC_WALLET")? {
        config.capabilities.vc_wallet = value;
    }
    if let Some(value) = env_bool("CHAIN_SDK_ISSUER_SIGNING")? {
        config.capabilities.issuer_signing = value;
    }
    if let Some(value) = env_string("CHAIN_SDK_CENTRALIZED_ENDPOINT") {
        config.centralized_endpoint = value;
    }
    if let Some(value) = env_string("CHAIN_SDK_CHAIN_ENDPOINT") {
        config.chain_endpoint = value;
    }
    if let Some(value) = env_bool("CHAIN_SDK_REQUIRE_FINALIZED")? {
        config.require_finalized_node = value;
    }
    if let Some(value) = env_parse::<u64>("CHAIN_SDK_CACHE_TTL_SECS")? {
        config.cache_ttl_secs = value;
    }
    if let Some(value) = env_string("CHAIN_SDK_DID_VC_STORAGE_PATH") {
        config.did.vc_storage_path = PathBuf::from(value);
    }
    if let Some(value) = env_string("CHAIN_SDK_MOCK_STATE_PATH") {
        config.mock.state_path = Some(PathBuf::from(value));
    }
    Ok(())
}

fn apply_overrides(config: &mut ChainSdkConfig, overrides: ConfigOverrides) {
    if let Some(value) = overrides.mode.as_deref() {
        // 非法值在 apply 阶段无法返回；由 resolve_config 预先校验。
        if let Ok(mode) = ChainSdkMode::parse(value) {
            config.mode = mode;
        }
    }
    if let Some(value) = overrides.role.as_deref() {
        if let Ok(role) = SdkRole::parse(value) {
            config.role = role;
        }
    }
    if let Some(value) = overrides.participant_id {
        config.participant_id = value;
    }
    if let Some(value) = overrides.controller_did {
        config.participant.controller_did = value;
    }
    if let Some(value) = overrides.key_vault_scheme {
        config.key_vault.scheme = value;
    }
    if let Some(value) = overrides.key_vault_path {
        config.key_vault.path = value;
    }
    if let Some(value) = overrides.key_vault_password {
        config.resolved_password = Some(SecretString::from(value));
    }
    if let Some(value) = overrides.key_vault_password_file {
        config.key_vault.password_file = value;
    }
    if let Some(value) = overrides.operator_key_id {
        config.key_vault.keys.operator = value;
    }
    if let Some(value) = overrides.participant_did_key_id {
        config.key_vault.keys.participant_did = value;
    }
    if let Some(value) = overrides.vc_wallet {
        config.capabilities.vc_wallet = value;
    }
    if let Some(value) = overrides.issuer_signing {
        config.capabilities.issuer_signing = value;
    }
    if let Some(value) = overrides.centralized_endpoint {
        config.centralized_endpoint = value;
    }
    if let Some(value) = overrides.chain_endpoint {
        config.chain_endpoint = value;
    }
    if let Some(value) = overrides.require_finalized_node {
        config.require_finalized_node = value;
    }
    if let Some(value) = overrides.cache_ttl_secs {
        config.cache_ttl_secs = value;
    }
    if let Some(value) = overrides.did_vc_storage_path {
        config.did.vc_storage_path = value;
    }
    if let Some(value) = overrides.mock_state_path {
        config.mock.state_path = Some(value);
    }
}

/// 解析配置。
///
/// 完整算法见 `docs/ChainSdk-Interface-Contract.md` §8。Issuer 在
/// `scheme = "file-keystore"` 时会解析密码；`memory-keystore` 不需要密码。
pub fn resolve_config(overrides: ConfigOverrides) -> Result<ChainSdkConfig, ChainSdkError> {
    // 先校验 override 中的枚举值，避免被 apply_overrides 静默忽略。
    if let Some(value) = overrides.mode.as_deref() {
        ChainSdkMode::parse(value)?;
    }
    if let Some(value) = overrides.role.as_deref() {
        SdkRole::parse(value)?;
    }

    let mut config = ChainSdkConfig::default();
    apply_file(&mut config)?;
    apply_env(&mut config)?;
    apply_overrides(&mut config, overrides);

    config.capabilities = config.capabilities.effective_for(config.role);

    if config.role.needs_keyvault() {
        crate::keys::validate_configured_keys(&config.key_vault.keys)?;
        if config.key_vault.scheme == "file-keystore" && config.resolved_password.is_none() {
            config.resolved_password = Some(resolve_keyvault_password(&config.key_vault)?);
        }
    } else {
        config.resolved_password = None;
    }

    validate_for_resolve(&config)?;
    Ok(config)
}

fn resolve_keyvault_password(config: &KeyVaultConfig) -> Result<SecretString, ChainSdkError> {
    if let Ok(value) = std::env::var(&config.password_env) {
        if !value.is_empty() {
            return Ok(SecretString::from(value));
        }
    }
    if config.password_file.exists() {
        return resolve_password(&PasswordSource::File(config.password_file.clone()))
            .map_err(map_vault_err);
    }
    if config.allow_auto_generate_password {
        if let Some(parent) = config.password_file.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                ChainSdkError::ConfigError(format!(
                    "create password dir {}: {error}",
                    parent.display()
                ))
            })?;
        }
        return resolve_password(&PasswordSource::AutoGenerate {
            target_file: config.password_file.clone(),
        })
        .map_err(map_vault_err);
    }
    Err(ChainSdkError::ConfigError(format!(
        "keystore password unavailable: set {} or create {}",
        config.password_env,
        config.password_file.display()
    )))
}

fn build_vault(config: &ChainSdkConfig) -> Result<Arc<dyn KeyVault>, ChainSdkError> {
    match config.key_vault.scheme.as_str() {
        "file-keystore" => {
            let password = config.resolved_password.clone().ok_or_else(|| {
                ChainSdkError::ConfigError(format!(
                    "keystore password was not resolved; set {} or call resolve_config()",
                    config.key_vault.password_env
                ))
            })?;
            let vault = FileKeystore::new(config.key_vault.path.clone(), password)
                .map_err(map_vault_err)?;
            Ok(Arc::new(vault))
        }
        "memory-keystore" => Ok(Arc::new(MemoryKeystore::new())),
        other => Err(ChainSdkError::ConfigError(format!(
            "unsupported key_vault.scheme: {other}"
        ))),
    }
}

fn validate_for_resolve(config: &ChainSdkConfig) -> Result<(), ChainSdkError> {
    if config.role.needs_keyvault() {
        crate::keys::validate_configured_keys(&config.key_vault.keys)?;
    }
    match config.mode {
        ChainSdkMode::Centralized if config.centralized_endpoint.trim().is_empty() => Err(
            ChainSdkError::ConfigError("mode centralized requires centralized_endpoint".into()),
        ),
        ChainSdkMode::Chain if config.chain_endpoint.trim().is_empty() => Err(
            ChainSdkError::ConfigError("mode chain requires chain_endpoint".into()),
        ),
        _ => Ok(()),
    }
}

fn validate_for_build(config: &ChainSdkConfig) -> Result<(), ChainSdkError> {
    validate_for_resolve(config)?;
    if config.participant.controller_did.trim().is_empty() {
        return Err(ChainSdkError::ConfigError(
            "participant.controller_did is required (did-sdk needs a participant DID)".into(),
        ));
    }
    Ok(())
}

/// 装配三个后端共享的 [`SdkCore`]。
///
/// - 收敛能力开关（Verifier 强制 `issuer_signing = false`）；
/// - Issuer 加载 `KeyVault` 与两类密钥（前缀校验，S14）；
/// - Verifier 不加载 `KeyVault`（S17）。
fn assemble_core(config: &ChainSdkConfig) -> Result<SdkCore, ChainSdkError> {
    let mut config = config.clone();
    config.capabilities = config.capabilities.effective_for(config.role);
    validate_for_build(&config)?;

    let vault = match config.role {
        SdkRole::Issuer => Some(build_vault(&config)?),
        SdkRole::Verifier => None,
    };
    let keys = match (&vault, config.role) {
        (Some(vault), SdkRole::Issuer) => Some(crate::keys::load_keys(
            vault.as_ref(),
            &config.key_vault.keys,
        )?),
        _ => None,
    };

    let did = Some(Arc::new(crate::did_adapter::DidSdkAdapter::build(&config)?)
        as Arc<dyn crate::did_adapter::DidPort>);

    let role = config.role;
    Ok(SdkCore {
        vault,
        did,
        config,
        role,
        keys,
    })
}

/// 按配置构造 `Arc<dyn ChainSdk>`。
///
/// - `Mock`：返回 [`crate::MockChainSdk`]（读、持久化、故障注入；写操作按 P2 推进）；
/// - `Centralized` / `Chain`：当前返回占位实现（P5 / P10 替换）。
///
/// Issuer 会加载 `KeyVault` 与两类密钥；Verifier 不加载 `KeyVault`（S17）。
pub fn build_chain_sdk(config: &ChainSdkConfig) -> Result<Arc<dyn ChainSdk>, ChainSdkError> {
    let core = assemble_core(config)?;
    match core.config.mode {
        ChainSdkMode::Mock => Ok(Arc::new(crate::mock::MockChainSdk::from_core(core)?)),
        ChainSdkMode::Centralized => Ok(Arc::new(crate::centralized::CentralizedChainSdk::new(
            core,
        )?)),
        ChainSdkMode::Chain => Ok(Arc::new(crate::chain::ChainModeSdk::new(core))),
    }
}

/// 构造具体的 Mock 后端（故障注入、状态断言等测试需要具体类型）。
pub fn build_mock_chain_sdk(
    config: &ChainSdkConfig,
) -> Result<Arc<crate::mock::MockChainSdk>, ChainSdkError> {
    let core = assemble_core(config)?;
    Ok(Arc::new(crate::mock::MockChainSdk::from_core(core)?))
}

/// 构造具体的 Chain 后端（测试需要具体类型时使用）。
pub fn build_chain_mode_sdk(
    config: &ChainSdkConfig,
) -> Result<Arc<crate::chain::ChainModeSdk>, ChainSdkError> {
    let core = assemble_core(config)?;
    Ok(Arc::new(crate::chain::ChainModeSdk::new(core)))
}

/// 构造具体的 Centralized 后端（测试需要具体类型时使用）。
pub fn build_centralized_chain_sdk(
    config: &ChainSdkConfig,
) -> Result<Arc<crate::centralized::CentralizedChainSdk>, ChainSdkError> {
    let core = assemble_core(config)?;
    Ok(Arc::new(crate::centralized::CentralizedChainSdk::new(
        core,
    )?))
}

/// 用调用方提供的状态构造 Mock 后端（fixtures / 测试）。
pub fn build_mock_chain_sdk_with_state(
    config: &ChainSdkConfig,
    state: crate::mock::MockState,
) -> Result<Arc<crate::mock::MockChainSdk>, ChainSdkError> {
    let core = assemble_core(config)?;
    Ok(Arc::new(crate::mock::MockChainSdk::with_state(core, state)))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    fn temp_vc_path() -> PathBuf {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!("chain-sdk-core-{}-{seq}", std::process::id()))
    }

    fn base_config(role: SdkRole) -> ChainSdkConfig {
        ChainSdkConfig {
            mode: ChainSdkMode::Mock,
            role,
            participant: ParticipantConfig {
                controller_did: "did:web:participant.example.com".into(),
            },
            did: DidConfig {
                vc_storage_path: temp_vc_path(),
            },
            key_vault: KeyVaultConfig {
                scheme: "memory-keystore".into(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn verifier_core_has_did_but_no_vault_or_keys() {
        let core = assemble_core(&base_config(SdkRole::Verifier)).unwrap();
        assert!(core.did.is_some(), "Verifier must assemble did-sdk (S17)");
        assert!(core.vault.is_none(), "Verifier must not load KeyVault");
        assert!(core.keys.is_none());
        assert!(!core.config.capabilities.issuer_signing);
    }

    #[test]
    fn issuer_core_has_vault_did_and_keys() {
        let core = assemble_core(&base_config(SdkRole::Issuer)).unwrap();
        assert!(core.did.is_some());
        assert!(core.vault.is_some());
        assert!(core.keys.is_some());
        assert!(core.config.capabilities.did_resolution);
    }

    #[test]
    fn build_requires_participant_did() {
        let mut config = base_config(SdkRole::Verifier);
        config.participant.controller_did.clear();
        let error = assemble_core(&config).err().expect("expected error");
        assert!(matches!(error, ChainSdkError::ConfigError(_)));
    }
}
