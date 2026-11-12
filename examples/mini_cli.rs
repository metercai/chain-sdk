//! `chain-sdk` 最小命令行工具。
//!
//! ```text
//! mini_cli init-keys                  # 在配置的 FileKeystore 中生成两类密钥并打印 KeyId
//! mini_cli health                     # health_check
//! mini_cli get-anchor <anchor_id>     # 查询锚定
//! mini_cli create-anchor <type> <payload_hash> [valid_until]
//! ```
//!
//! 配置解析顺序见 `docs/ChainSdk接口契约与实现规格书.md` §8；可用
//! `CHAIN_SDK_HOME` / `CHAIN_SDK_CONFIG_FILE` / 环境变量覆盖。

use std::process::ExitCode;

use chain_sdk::config::{ConfigOverrides, resolve_config};
use chain_sdk::{AnchorType, build_chain_sdk};
use keyvault_core::{FileKeystore, KeyAlgorithm, KeyNamespace, KeyVault};
use secrecy::ExposeSecret;

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "health".to_string());
    match command.as_str() {
        "init-keys" => init_keys().await,
        "health" => {
            let sdk = sdk()?;
            let health = sdk
                .health_check()
                .await
                .map_err(|error| error.to_string())?;
            print_json(&health)
        }
        "get-anchor" => {
            let anchor_id = args
                .next()
                .ok_or_else(|| "usage: mini_cli get-anchor <anchor_id>".to_string())?;
            let sdk = sdk()?;
            let anchor = sdk
                .get_anchor(&anchor_id)
                .await
                .map_err(|error| error.to_string())?;
            print_json(&anchor)
        }
        "create-anchor" => {
            let anchor_type = args.next().ok_or_else(|| {
                "usage: mini_cli create-anchor <type> <payload_hash> [valid_until]".to_string()
            })?;
            let payload_hash = args.next().ok_or_else(|| {
                "usage: mini_cli create-anchor <type> <payload_hash> [valid_until]".to_string()
            })?;
            let valid_until = args
                .next()
                .map(|value| value.parse::<u64>())
                .transpose()
                .map_err(|error| format!("valid_until: {error}"))?
                .unwrap_or(0);
            let sdk = sdk()?;
            let anchor_id = sdk
                .create_anchor(parse_anchor_type(&anchor_type), &payload_hash, valid_until)
                .await
                .map_err(|error| error.to_string())?;
            println!("{anchor_id}");
            Ok(())
        }
        "help" | "--help" | "-h" => {
            println!(
                "mini_cli <init-keys|health|get-anchor|create-anchor>\n\
                 env: CHAIN_SDK_HOME, CHAIN_SDK_CONFIG_FILE, CHAIN_SDK_MODE, CHAIN_SDK_ROLE, ..."
            );
            Ok(())
        }
        other => Err(format!("unknown command: {other}")),
    }
}

fn sdk() -> Result<std::sync::Arc<dyn chain_sdk::ChainSdk>, String> {
    let config = resolve_config(ConfigOverrides::default()).map_err(|error| error.to_string())?;
    build_chain_sdk(&config).map_err(|error| error.to_string())
}

async fn init_keys() -> Result<(), String> {
    let config = resolve_config(ConfigOverrides {
        role: Some("issuer".into()),
        ..Default::default()
    })
    .map_err(|error| error.to_string())?;

    if config.key_vault.scheme != "file-keystore" {
        return Err("init-keys requires key_vault.scheme = \"file-keystore\"".into());
    }
    let password = config
        .resolved_password
        .clone()
        .ok_or_else(|| "keystore password was not resolved".to_string())?;
    let vault = FileKeystore::new(config.key_vault.path.clone(), password)
        .map_err(|error| error.to_string())?;

    let chain_key = vault
        .generate(KeyNamespace::ChainAccount, KeyAlgorithm::Sr25519)
        .await
        .map_err(|error| error.to_string())?;
    let did_key = vault
        .generate(KeyNamespace::ParticipantDid, KeyAlgorithm::Ed25519)
        .await
        .map_err(|error| error.to_string())?;

    tracing::info!(
        path = %config.key_vault.path.display(),
        "generated service keys (update your config with the KeyIds below)"
    );
    println!("[chain_sdk.key_vault.keys]");
    println!("operator = \"{}\"", chain_key.0);
    println!("participant_did = \"{}\"", did_key.0);
    let _ = config
        .resolved_password
        .as_ref()
        .map(ExposeSecret::expose_secret);
    Ok(())
}

fn parse_anchor_type(value: &str) -> AnchorType {
    match value {
        "EmailBinding" => AnchorType::EmailBinding,
        "DomainControl" => AnchorType::DomainControl,
        "DidDocument" => AnchorType::DidDocument,
        "Warrant" => AnchorType::Warrant,
        "Delegation" => AnchorType::Delegation,
        "OrgMembership" => AnchorType::OrgMembership,
        "IssuedVc" => AnchorType::IssuedVc,
        "GatewayAccess" => AnchorType::GatewayAccess,
        other => AnchorType::Custom(other.to_string()),
    }
}

fn print_json<T: serde::Serialize>(value: &T) -> Result<(), String> {
    let json = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    println!("{json}");
    Ok(())
}
