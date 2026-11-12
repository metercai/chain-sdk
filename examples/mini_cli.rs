//! 初始化占位命令行工具。
//!
//! P0–P11 完成后提供：
//!
//! ```text
//! mini_cli init-keys      # 在配置的 KeyVault 中生成两类密钥（POC/测试）
//! mini_cli health         # health_check
//! mini_cli get-anchor ID  # 查询锚定
//! mini_cli create-anchor  # 创建锚定（Issuer）
//! ```
//!
//! 当前只打印版本与配置根目录，用于验证依赖图与 musl 静态链接。

fn main() {
    println!(
        "chain-sdk {} (initialization skeleton)",
        env!("CARGO_PKG_VERSION")
    );
    println!("config home: {}", chain_sdk::config::home_dir().display());
    println!("status: implementation pending review (docs/implementation-plan.md)");
}
