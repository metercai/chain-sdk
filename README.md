# chain-sdk

链能力契约（`Mock` / `Centralized` / `Chain` 三后端 × `Verifier` / `Issuer` 两角色），
为服务端 Rust 应用提供**锚定读写、参与方查询、DID 解析、断言签名与事件订阅**。

> **当前状态：P0 完成（v0.1.0），按 [`docs/implementation-plan.md`](docs/implementation-plan.md) 推进。**
>
> - 已完成（P0–P3）：公开契约、配置解析（TOML / env / override / 密码链）、
>   两类密钥 namespace 校验（S14）、角色门禁（`ensure_issuer` / `ensure_self` /
>   `ensure_signer`）；
> - 已完成（P1–P2）：`Mock` 后端——读、写（锚定生命周期 / 自身管理写）、
>   双向索引、JSON 持久化、故障注入，以及 S9/S19 越权拦截；
> - 已完成（P3–P4）：`did_adapter` 按角色装配 did-sdk（两角色均有 did-resolution，
>   仅 Issuer 加载 KeyVault 与两类密钥）；`did:key` 离线解析、`did:web` 委派 did-sdk；
> - 已完成（P5–P6）：`Centralized` 后端——HTTP 只读、`get_anchor` TTL 缓存、
>   写请求签名（`op || "\n" || canonical JSON`）与 `x-chain-*` 头；
> - 已完成（P7–P8）：参与方三段式绑定、委托锚定 fixtures、`business_types`
>   不参与鉴权、无治理接口与无 VC 存储方法检查；
> - 已完成（P9）：Mock 内存事件与 Centralized WebSocket 事件（Chain 在 P10 接入）；
> - 进行中（P10）：Chain 后端（subxt 连接/健康检查/事件 + 角色拦截）；
> - 本地门禁：77 个测试通过；
> - GitHub 仓库尚未创建（用户选择先本地评审，见实施文档 D-18）。

| 项目 | 内容 |
|---|---|
| 仓库 | `https://github.com/metercai/chain-sdk`（待创建/推送） |
| 语言 | Rust 1.88.0（`edition 2024`，`resolver = "3"`） |
| 依赖 | `keyvault-core` / `did-sdk` 均为**外部依赖**（`did-sdk` 仓库 `v0.1.0` tag） |
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
| `Chain` | `subxt` storage 只读；无 `KeyVault` | `subxt` 读写；sr25519 签 extrinsic + ed25519 签断言 |

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
│   ├── lib.rs            # ChainSdk trait（公开契约）
│   ├── config.rs         # 配置模型 + 工厂入口（待实现）
│   ├── types.rs          # 数据模型
│   ├── error.rs          # ChainSdkError + map_vault_err
│   └── events.rs         # EventData / Unsubscribe
├── examples/mini_cli.rs
├── tests/                # 15 个验收测试文件（占位）
├── configs/chain-sdk.toml.example
├── scripts/{build-rust-musl,run-consistency,verify-roles}.sh
├── .github/workflows/{ci,rust-musl,consistency,roles}.yml
├── metadata/README.md    # 为何不提交 identity_chain.scale
└── docs/
    ├── implementation-plan.md      # 实施步骤（评审入口）
    ├── acceptance-checklist.md     # S1–S20 + SD-JWT 验收清单
    └── specs/                      # 三份输入规格快照
```

---

## 10. VC 选择性披露（SD-JWT，规划中）

`docs/specs/03-VC选择性披露特性补充附件.md` 定义在**不修改链上锚定原语**的前提下，
用 SD-JWT 盐化哈希为 VC 增加选择性披露能力：

- VC 的 `credentialSubject` 增加 `_sd`（盐化哈希数组）与 `_sd_alg`；
- 发行方生成 `[salt, name, value]` Disclosures，与 VC 一并交付持有者；
- 持有者构造 VP 时附带 `disclosures` 子集；
- 验证方重算哈希并与 `_sd` 比对，再查链上锚定。

本仓库的实现范围（评审决策 D-13～D-16）：

- **Issuer**：盐化哈希 + Disclosures 生成；
- **Verifier**：Disclosure 哈希比对 + `get_anchor` 生命周期检查；
- **did-sdk**：Disclosures 存储、用户选择与 VP 构造（**跨仓库**，需 did-sdk v0.2）。

当前 `did-sdk 0.1.0` 的 `VerifiableCredential` 没有 `credentialStatus` 字段，
`VerifiablePresentation` 没有 `disclosures` 字段，VP 签名输入也不覆盖 disclosures。
因此端到端选择性披露**依赖 did-sdk 模型扩展**；`chain-sdk` 先以自包含的
`SdCredential` / `SdPresentation` JSON 契约实现签发与验证原语，并在实施文档中
记录跨仓库依赖。

---

## 11. 安全说明

- 私钥永不导出：`KeyVault` 只提供 `sign` / `public_key`；
- 两层 namespace 校验：`chain-sdk::keys` 前缀校验 + `keyvault-core` 内部校验；
- 配置与日志不包含密码、私钥、密文；`SecretString` 使用 redacted `Debug`；
- 写请求体签名覆盖 `op + canonical JSON`，通过 `x-chain-*` 头传递；
- `UserRejected` 从 `KeyVaultError::UserRejected` 1:1 透传，不做字符串比较；
- 验证方不因 `did-sdk` 的锚定预览结果做验证决策（R1 修订）。

---

## 12. 文档索引

| 文档 | 说明 |
|---|---|
| [`docs/implementation-plan.md`](docs/implementation-plan.md) | **实施步骤与评审决策（评审入口）** |
| [`docs/acceptance-checklist.md`](docs/acceptance-checklist.md) | S1–S20 与 SD-JWT 验收项 |
| [`docs/specs/01-...`](docs/specs/) | ChainSdk 接口契约与实现规格书 v2 |
| [`docs/specs/02-...`](docs/specs/) | did-sdk 移除链上查询与 subxt 依赖的修订（R1） |
| [`docs/specs/03-...`](docs/specs/) | VC 选择性披露特性补充附件 |

---

## 13. 未来扩展

- `keyvault-device` / `keyvault-server` 通过 `keyvault.scheme` 切换，只新增 crate +
  在 `build_vault` 加 feature 门控分支；`ChainSdk` trait 与 `SdkRole` 不改动；
- `chain-owner-sdk` 复用本 crate 的读能力与 `keys::sign_extrinsic`，扩展治理接口；
- BBS+ 作为选择性披露的并存机制（`_bbs` 字段），不替换 SD-JWT。

## 14. 许可

Apache-2.0，见 [LICENSE](LICENSE)。
