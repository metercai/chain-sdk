# chain-sdk

链能力契约（`Mock` / `Centralized` / `Chain` 三后端 × `Verifier` / `Issuer` 两角色），
为服务端 Rust 应用提供**锚定读写、参与方查询、DID 解析、断言签名与事件订阅**。

> **当前状态：v0.1.1（增量的宿主集成修订），已完成实现并通过 CI。**
>
> - 契约与实现：[`docs/ChainSdk-Interface-Contract.md`](docs/ChainSdk-Interface-Contract.md)（唯一规范文档）
> - 本地门禁：122 个测试通过；`cargo fmt --check`、`cargo clippy -D warnings` 干净
> - CI：`ci` / `rust-musl` / `consistency` / `roles` 四个 workflow 全绿
> - did-sdk contract: [DidSdk Interface Contract](https://github.com/metercai/did-sdk/blob/main/docs/DidSdk-Interface-Contract.md)
> - 依赖：`did-sdk` / `keyvault-core` 固定 **v0.2.1**；选择性披露与 did-sdk 的
>   `Disclosure` / `verify_vp` / `VP-SD-v1` 对齐，VC 哈希复用 did-sdk 公开 API

| 项目 | 内容 |
|---|---|
| 仓库 | [`metercai/chain-sdk`](https://github.com/metercai/chain-sdk)（`main` 已推送，4 个 workflow 全绿） |
| 语言 | Rust 1.88.0（`edition 2024`，`resolver = "3"`） |
| 依赖 | `keyvault-core` / `did-sdk` 均为**外部依赖**（`did-sdk` 仓库 `v0.2.1` tag） |
| 跨平台 | 四目标矩阵，统一由 GitHub Actions runner 编译 |
| 绑定 | **不提供** Python / Node / WASM 绑定 |

---

## 1. 职责边界

```text
chain-owner-sdk（治理：注册参与方 / 注册锚定类型 / 委托锚定提交）
        │ 依赖
        ▼
chain-sdk（本仓库：锚定读、锚定写、参与方查询、DID 解析、断言签名）
        │ 依赖
        ├── did-sdk（DID 方法 + VC 钱包；无链上查询、无 subxt）
        └── keyvault-core（KeyVault trait / FileKeystore / MemoryKeystore）
```

- **锚定查询在本 crate**：`get_anchor` / `list_anchors` 是验证方核对锚定状态的入口；
  `did-sdk` 不提供链上查询（R1 修订）。
- **VC 存储不在本 crate**：`store_vc` / `get_vc` / `list_vcs` / `remove_vc` 由
  `did-sdk<Server>` 持有侧管理（S17）。本 crate 的 `ChainSdk` trait **不含**这些方法。
- **治理接口不在本 crate**：`register_participant` / `deactivate_participant` /
  `update_operator` / `register_anchor_type` / `submit_delegated_anchor` 属于
  `chain-owner-sdk`（S18）。CI 用 `grep` 断言。
- **不提供 Python / Node / WASM 绑定**；若未来需要，作为独立仓库。

---

## 2. 角色模型：`Verifier ⊂ Issuer`

`chain-sdk` 只暴露两种**配置角色**（`SdkRole`）：

| 角色 | 能力 | 密钥 |
|---|---|---|
| `Verifier` | 只读：锚定、参与方、DID、事件 | **不加载** `KeyVault` |
| `Issuer` | Verifier 全部读能力 + 锚定写 + 自身管理写 + 断言签名 | 加载 `chain-account/` + `participant-did/` |

- **线性包含**：`Verifier ⊂ Issuer`；不存在“纯 Issuer 不能验证”的情况。
- **旧配置 `role = "app"`** 通过 serde `alias` 映射为 `Issuer`，不是独立变体。
- **`ChainRole` 与 `SdkRole` 是两个概念**：
  - `ChainRole`（`Verifier` / `Issuer` / `ChainOwner`）是**链上注册表字段**，
    由链主治理分配，决定链上能力边界；
  - `SdkRole` 是 **SDK 实例化配置**，决定本进程加载哪些密钥与能力。

### 三后端 × 两角色

| 模式 \ 角色 | `Verifier` | `Issuer` |
|---|---|---|
| `Mock` | 内存读；无 `KeyVault`；断言签名返回 `SignerUnavailable` | 内存读写；两类密钥 |
| `Centralized` | HTTP 只读；无 `KeyVault` | HTTP 读写；链账户签名 + did:web 断言签名 |
| `Chain` | 连接/健康/事件/角色拦截；storage 查询 POC 返回 `Unsupported`；无 `KeyVault` | 上述能力 + 写操作角色拦截；extrinsic 提交 POC 返回 `Unsupported`（两类签名原语已实现） |

**越权行为（三后端一致）**：

| 调用 | Verifier | Issuer |
|---|---|---|
| 写方法（`create_anchor` 等） | `Unauthorized` | 允许（Chain 未实现时 `Unsupported`） |
| `update_*` 修改他人 | `Unauthorized` | `Unauthorized`（S19） |
| `sign_participant_assertion` | `SignerUnavailable` | 允许 |

---

## 3. 两类服务端密钥与 namespace 隔离（S14）

| namespace | 算法 | 用途 | 唯一入口 |
|---|---|---|---|
| `chain-account/` | sr25519 | extrinsic 签名、HTTP 写请求体签名 | `keys::sign_extrinsic` |
| `participant-did/` | ed25519 | 参与方 `did:web` 断言签名 | `keys::sign_participant_assertion_inner` |

- 两个函数是**唯一入口**，返回类型不同（`Sr25519Signature` vs `Ed25519Signature`），
  类型级不可互换；
- 入口处强制校验 `KeyId` 前缀，不匹配返回
  `ChainSdkError::KeyNamespaceViolation`；
- `keyvault-core` 自身也校验 namespace + algorithm，两层防护。

**密钥 provisioning**：`keyvault-core` 的 `generate(namespace, algorithm)` 会按公钥
生成 `KeyId`（sr25519 为 `chain-account/<SS58>`，ed25519 为
`participant-did/did_key_<multibase>`）。配置中的 `operator` / `participant_did`
必须指向**已存在**的 `KeyId`；`chain-sdk` 不会静默创建身份密钥。

---

## 4. 参与方身份三段式硬绑定（S15）

`ParticipantRecord`：

```text
controller_did（did:web，身份根） ↔ participant_id（链上主键） ↔ operator（SS58 链账户）
```

- Mock/Centralized 维护 `did_participant` 与 `account_participant` 双向索引；
- `get_participant_by_did` 先查 DID 索引，再取记录；
- `update_controller_did` 同步维护双向索引。

### 委托锚定语义（S16）

`AnchorRecord` 三字段分离：

| 字段 | 语义 |
|---|---|
| `issuer_did` | **语义签发者 = 信任链根**（委托锚定时为被代理方） |
| `submitter_id` | 实际提交者（治理审计） |
| `delegated_by_did` | 委托锚定的代理方（透明审计） |

`chain-sdk` 的 `create_anchor` 由 Issuer 自身提交：`issuer_did = controller_did`、
`delegated_by_did = None`。委托锚定由 `chain-owner-sdk.submit_delegated_anchor`
提交，本 crate 只读。

### `business_types` 不参与鉴权（S20）

权限判定只依赖 `chain_role` + `authorized_domains`；`role_gate.rs` 不读取
`business_types`。

---

## 5. 按角色装配能力（S17）

| 能力 | Verifier | Issuer | 配置项 |
|---|---|---|---|
| `did-resolution` | 是 | 是 | `capabilities.did_resolution` |
| `vc-wallet` | 可选 | 可选 | `capabilities.vc_wallet`（POC 默认 `false`） |
| `issuer-signing` | **否** | **是** | `capabilities.issuer_signing`（Verifier 强制 `false`） |
| `KeyVault` | 不加载 | 加载两类密钥 | — |
| 写操作 | 无 | 锚定写 + 自身管理写 + 断言签名 | — |

`did-sdk` 的装配由 `src/did_adapter.rs` 内部决定，**不把** `Client`/`Server`
模式选择暴露给 `chain-sdk` 配置。`did-sdk<Server>` 以 `key_vault = None` 构造：
参与方签名私钥只在 `chain-sdk` 侧，经 namespace 校验后使用（S14/S17）。

---

## 6. 配置与环境变量

优先级：**显式 `ConfigOverrides` > 环境变量 > TOML 文件 > 内置默认值**。
密码单独一条链（仅 `role = "issuer"` 时解析）：
`CHAIN_KEYSTORE_PASSWORD` → `password_file` → `allow_auto_generate_password`
自动生成 → 报错。

| 环境变量 | 覆盖项 |
|---|---|
| `CHAIN_SDK_HOME` / `CHAIN_SDK_CONFIG_FILE` | 根目录 / 配置路径 |
| `CHAIN_SDK_MODE` | `mock` / `centralized` / `chain` |
| `CHAIN_SDK_ROLE` | `verifier` / `issuer`（`app` 为别名） |
| `CHAIN_SDK_PARTICIPANT_ID` | `participant_id` |
| `CHAIN_SDK_CONTROLLER_DID` | `participant.controller_did` |
| `CHAIN_KEYSTORE_PASSWORD` | 密钥库密码（仅 Issuer） |
| `CHAIN_SDK_KEYVAULT_SCHEME` / `_PATH` / `_PASSWORD_FILE` | KeyVault 配置 |
| `CHAIN_SDK_OPERATOR_KEY_ID` / `_PARTICIPANT_DID_KEY_ID` | 两类密钥的 `KeyId` |
| `CHAIN_SDK_VC_WALLET` / `_ISSUER_SIGNING` | 能力开关 |
| `CHAIN_SDK_CENTRALIZED_ENDPOINT` / `_CHAIN_ENDPOINT` | 端点 |
| `CHAIN_SDK_REQUIRE_FINALIZED` / `_CACHE_TTL_SECS` | 行为 |
| `CHAIN_SDK_DID_VC_STORAGE_PATH` | `did-sdk` 的 VC 存储目录 |

完整样例见 [`configs/chain-sdk.toml.example`](configs/chain-sdk.toml.example)。
模式切换（`CHAIN_SDK_MODE`）需要重启进程。

> **注意**：`did-sdk<Server>` 构造时会创建 VC 存储目录（`vc_storage_path`）及其
> `.vc_password` / `.vc_meta` 文件，即使 `vc_wallet = false`。这是 `did-sdk` 的
> 存储初始化行为，`chain-sdk` 不读写这些文件。

---

### 6.3 宿主集成 API（v0.1.1）

供 `chain-owner-sdk` 等宿主复用同一份密钥库与 Mock 状态；均为增量 API，旧构造函数不变。

| API | 用途 |
|---|---|
| `build_key_vault(&cfg)` | 按配置只打开一次 `KeyVault` |
| `build_chain_sdk_with_vault(&cfg, vault)` | 用注入的 vault 构造 `Arc<dyn ChainSdk>` |
| `build_{mock,centralized,chain_mode}_*_with_vault` | 具体后端变体 |
| `MockStateHandle::{new,snapshot,update}` | 共享 `Arc<RwLock<MockState>>` |
| `MockChainSdk::{state_handle,persist}` | 取得共享句柄 / 显式持久化 |
| `build_mock_chain_sdk_with_state_handle[_and_vault]` | 共享状态的 Mock 构造 |
| `canonical_json(&value)` | 公开的 canonical JSON（服务端逐字节验签） |

约束：

- vault 注入要求 `role = Issuer`，`Verifier + vault` 返回 `ConfigError`；
- 注入模式下 `key_vault.keys` 仍决定 `KeyId`，`scheme/path/password_file` 不再用于打开存储；
- 通过 `MockStateHandle` 做外部写不会自动发事件，宿主需显式 `persist()`；
- Centralized 写错误体按 `{code, message, request_id, details?}` 解析（16 KiB 上限），
  409 + `ANCHOR_PERMANENT` / `ANCHOR_REVOKED` 分别映射到对应错误变体；
- 线协议签名夹具见 [`tests/fixtures/centralized_wire.json`](tests/fixtures/centralized_wire.json)。

---

## 7. 平台支持与 musl 静态链接

| 目标 | Runner | 产物要求 |
|---|---|---|
| `x86_64-unknown-linux-musl` | `ubuntu-latest` | **完全静态**（`file` / `ldd` 断言） |
| `aarch64-unknown-linux-musl` | `ubuntu-latest` | **完全静态** |
| `aarch64-apple-darwin` | `macos-15` | 原生 |
| `x86_64-pc-windows-msvc` | `windows-latest` | 原生 |

- **跨平台编译只在 GitHub Actions runner 执行**（本地不做交叉编译）；
- TLS 统一 `rustls`（纯 Rust）：`reqwest(rustls-tls)` +
  `tokio-tungstenite(rustls-tls-webpki-roots)`；
- 禁止 `native-tls` / `openssl` / 未 bundled 的 `-sys` 依赖；
- `ring` 是 `rustls` 的传递依赖（纯 Rust + 汇编，无系统库），被接受；
- `subxt` 为纯 Rust，保留在 `chain-sdk`（`did-sdk` 依赖树中不出现 `subxt`）。

---

## 8. 快速开始

```bash
# 工具链：rust-toolchain.toml 固定 1.88.0
cargo check --locked --all-targets --all-features
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
cargo run --locked --example mini_cli
```

> **沙箱环境提示**（本机）：`~/.cargo` 与 `~/.rustup` 不可写时，使用仓库内
> gitignored 的 `.cargo-home` / `.rustup` 与 `.cargo/config.toml`（Tuna 稀疏镜像、
> `git-fetch-with-cli`）。普通开发机与 CI 无需这些文件。

---

## 9. 仓库结构

```text
chain-sdk/
├── Cargo.toml / rust-toolchain.toml / .gitignore / README.md / LICENSE
├── src/
│   ├── lib.rs            # ChainSdk trait（17 方法）+ SdkCore
│   ├── config.rs         # 配置解析（TOML / env / overrides / 密码链）
│   ├── types.rs          # 数据模型
│   ├── error.rs          # ChainSdkError + map_vault_err
│   ├── events.rs         # 回调表 / EventData / Unsubscribe
│   ├── keys.rs           # 两类密钥 + namespace 唯一入口
│   ├── role_gate.rs      # ensure_issuer / ensure_self / ensure_signer
│   ├── did_adapter.rs    # DidPort（did-sdk v0.2.1 适配）
│   ├── canonical.rs      # 公开 canonical JSON（写请求签名）
│   ├── mock.rs           # Mock 后端 + MockStateHandle
│   ├── centralized.rs    # Centralized 后端（HTTP + WS + TTL）
│   ├── chain.rs          # Chain 后端（subxt）
│   └── sd.rs             # 选择性披露签发/验证
├── examples/mini_cli.rs
├── tests/                # 22 个验收测试文件 + fixtures
├── configs/chain-sdk.toml.example
├── scripts/{build-rust-musl,run-consistency,verify-roles}.sh
├── .github/workflows/{ci,rust-musl,consistency,roles}.yml
├── metadata/README.md    # 为何不提交 identity_chain.scale
└── docs/
    └── ChainSdk-Interface-Contract.md   # 唯一规范文档（英文）
```

---

## 10. VC 选择性披露（盐化哈希）

选择性披露在**不修改链上锚定原语**的前提下实现：链上仍然只存 VC 整体哈希。

| 产物 | 内容 |
|---|---|
| VC | `did_sdk::VerifiableCredential`，`credentialSubject._sd` + `_sd_alg`，锚定 id 在 `anchor_id` |
| Disclosures | `did_sdk::Disclosure { salt, name, value }`，与 VC 一并交付持有者 |
| VP | `did_sdk::VerifiablePresentation`，`disclosures: Some([...])` + holder proof |
| 锚定 | `payload_hash` 覆盖去掉 `proof` / `anchor_id` 的 VC，生命周期语义不变 |

- **Issuer**（本仓库）：生成 16 字节独立随机盐与 `_sd` 哈希、创建锚定、用
  `participant-did/` 密钥签发 VC；
- **Verifier**（本仓库）：复用 `did-sdk v0.2.1` 的 `verify_vp`（holder/VC/Disclosure），
  再检查锚定 `revoked` / `valid_until` / `payload_hash`；
- **持有者**（did-sdk v0.2.1）：`store_vc_with_disclosures`、`build_vp_selective` 构造
  `VP-SD-v1` 出示。

哈希公式与 `did-sdk` v0.2.1 完全一致：
`SHA-256(base64url(salt) || base64url(name) || base64url(value))`。
隐私边界（同一 VC 跨出示可关联、验证方合谋拼接不可防）见规格书 §7.7。

## 11. 安全说明

- 私钥永不导出：`KeyVault` 只提供 `sign` / `public_key`；
- 两层 namespace 校验：`chain-sdk::keys` 前缀校验 + `keyvault-core` 内部校验；
- 配置与日志不包含密码、私钥、密文；`SecretString` 使用 redacted `Debug`；
- 写请求体签名覆盖 `op + canonical JSON`，通过 `x-chain-*` 头传递；canonical 实现已公开
  (`chain_sdk::canonical_json`)，服务端可逐字节复现；
- `UserRejected` 从 `KeyVaultError::UserRejected` 1:1 透传，不做字符串比较；
- 验证方不因 `did-sdk` 的锚定预览结果做验证决策（R1 修订）。

---

## 12. 文档索引

| 文档 | 说明 |
|---|---|
| [`docs/ChainSdk-Interface-Contract.md`](docs/ChainSdk-Interface-Contract.md) | **唯一规范文档**：契约、后端、选择性披露、配置、验收、状态 |

## 13. 未来扩展

- `keyvault-device` / `keyvault-server` 通过 `keyvault.scheme` 切换，只新增 crate +
  在 `build_key_vault` 加 feature 门控分支；`ChainSdk` trait 与 `SdkRole` 不改动；
- BBS+ 作为选择性披露的并存机制（`_bbs` 字段），不替换当前选择性披露机制；
- 写请求时间戳/nonce（重放保护）与 Mock 外部事件注入在 v0.1.1 中暂缓。

## 14. 许可

Apache-2.0，见 [LICENSE](LICENSE)。
