# chain-sdk 实施步骤文档（评审稿）

| 项目 | 内容 |
|---|---|
| 文档编号 | CHAIN-SDK-PLAN-001 |
| 版本 | v0.1（评审稿） |
| 日期 | 2026-11-13 |
| 状态 | **待评审**：批准 §4 决策与 §11 确认清单后进入实现 |
| 输入规格 | `docs/specs/01-ChainSdk接口契约与实现规格书-v2.md`（下称「规格一」）<br>`docs/specs/02-did-sdk移除链上查询与subxt依赖的修订-R1.md`（「R1」）<br>`docs/specs/03-VC选择性披露特性补充附件.md`（「SD-JWT 附件」） |
| 初始化基线 | 本仓库首个提交（骨架 + 本文档） |
| 评审入口 | §4 关键决策（D-01～D-20）、§11 确认清单 |

> **阅读顺序**：先看 §3（架构与边界）与 §4（需要拍板的决策），再看 §5（分阶段步骤）、
> §10（风险）。§12–§15 是可直接执行的契约附录。
>
> **初始化原则**：本提交刻意**不写业务代码**——`src/` 只有公开契约与工厂占位，
> 测试文件只有占位注释，`mini_cli` 只打印版本。这样 `cargo check` 验证的是
> 依赖图、工具链与 API 形状，而不是未经评审的实现。

---

## 1. 目标、范围与成功定义

### 1.1 交付目标

单一 Rust crate `chain-sdk`，实现：

1. **三后端**：`Mock`（进程内）、`Centralized`（HTTP + WebSocket）、`Chain`（subxt）；
2. **两角色**：`Verifier`（只读）与 `Issuer`（读写 + 断言签名），线性包含；
3. **统一契约**：`ChainSdk` trait 的 16 个方法（S1）；
4. **两类服务端密钥 + namespace 隔离**（S14）；
5. **参与方身份三段式绑定 + 委托锚定语义**（S15/S16）；
6. **按角色装配 did-sdk 能力子集**（S17）；
7. **无治理接口**（S18）；
8. **VC 选择性披露原语**（SD-JWT 附件 I/V/P 组，评审决策后纳入）；
9. 四目标平台产物，跨平台编译由 GitHub Actions runner 执行。

### 1.2 成功定义（以命令为准）

| 编号 | 成功标准 | 验证命令 |
|---|---|---|
| A1 | 依赖图可解析、`cargo check` 通过 | `cargo check --locked --all-targets --all-features` |
| A2 | 格式 / Clippy 零告警 | `cargo fmt --all -- --check`；`cargo clippy --locked --all-targets --all-features -- -D warnings` |
| A3 | 全部测试通过 | `cargo test --locked --all-features` |
| A4 | S1–S20 逐项有证据 | `docs/acceptance-checklist.md` + `scripts/verify-roles.sh` |
| A5 | Linux musl 完全静态 | CI `rust-musl` job 的 `file` / `ldd` 断言 |
| A6 | 四平台矩阵全绿 | GitHub Actions `rust-musl` 四 target |
| A7 | 治理 / VC 存储边界 | `scripts/verify-roles.sh` 的 grep 断言 |
| A8 | did-sdk 不依赖 subxt（R1） | `cargo tree -p did-sdk` 无 `subxt`；`roles.yml` 断言 |
| A9 | SD-JWT 签发/验证/隐私验收（若批准） | `cargo test --features sd-jwt --test sd_*` |
| A10 | 文档与验收记录同步 | README / `docs/acceptance-checklist.md` |
| A11 | 4 个 workflow 在 GitHub 全绿 | Actions 运行记录 |
| A12 | 评审批准 D-01～D-20 | 本文件 §11 |

### 1.3 非目标（本期不做）

- Python / Node / WASM 绑定（规格明确不提供；未来如需为独立仓库）；
- `chain-owner-sdk` 的治理接口（本 crate 只读）；
- VC 的存储、列出、删除、VP 构造（属 `did-sdk`）；
- 生产级 HSM / 设备安全区（只保留 `device-keystore` / `server-keystore` feature 门控）；
- 真实 identity-chain 的集成验收（缺元数据与节点，见 D-06）；
- crates.io 发布（另立发布任务）；
- BBS+ 实现（SD-JWT 附件的后续升级路径，不在本期）。

---

## 2. 初始化现状（已完成，可复核）

### 2.1 已建立的仓库骨架

```text
chain-sdk/
├── Cargo.toml                          # 单 crate + workspace 根
├── Cargo.lock                          # 已生成（472 个包），将提交
├── rust-toolchain.toml                 # 1.88.0 + 四 target
├── .gitignore
├── LICENSE                             # Apache-2.0
├── README.md                           # 契约与边界说明
├── src/
│   ├── lib.rs                          # ChainSdk trait（16 方法，已冻结）
│   ├── config.rs                       # 配置模型 + resolve_config/build_chain_sdk 占位
│   ├── types.rs                        # ParticipantRecord / AnchorRecord / DidRecord ...
│   ├── error.rs                        # ChainSdkError（17 变体）+ map_vault_err
│   └── events.rs                       # EventData / Unsubscribe
├── examples/mini_cli.rs                # 占位 CLI
├── tests/                              # 15 个测试文件（占位）
│   ├── common/mod.rs
│   ├── configs/{verifier,issuer}.toml
│   └── fixtures/{mock_state,delegated_anchor}.json
├── configs/chain-sdk.toml.example
├── scripts/{build-rust-musl,run-consistency,verify-roles}.sh
├── .github/workflows/{ci,rust-musl,consistency,roles}.yml
├── metadata/README.md                  # 为何不提交 .scale
└── docs/
    ├── implementation-plan.md          # 本文档
    ├── acceptance-checklist.md         # S1–S20 + SD-JWT 清单
    └── specs/                          # 三份输入规格快照
```

### 2.2 工具链基线

| 项 | 初始化环境 | 说明 |
|---|---|---|
| OS | macOS arm64（10 核 / 16 GB） | 本地开发机 |
| Rust | `rustc 1.88.0` | `rust-toolchain.toml` 固定 1.88.0（D-01） |
| Cargo | 1.88.0 | `resolver = "3"` |
| 依赖缓存 | 仓库内 `.cargo-home`（gitignored） | 沙箱 `~/.cargo` 只读（D-19） |
| GitHub CLI | **无** | 无法创建仓库（D-18） |
| zig / cargo-zigbuild | **无** | 本地不做交叉编译（D-18） |

### 2.3 依赖解析结果（`Cargo.lock` 已生成，将提交）

规格一要求 Rust 1.85 + `subxt = "0.50"`，但：

- `did-sdk v0.1.0` 的 `rust-version = 1.88`；
- `subxt 0.50.3` 的 MSRV 为 **1.88**。

这是规格内部的硬冲突。初始化实测后把 MSRV 定为 **1.88.0**，`resolver = "3"`。关键版本：

| crate | 版本 | 备注 |
|---|---|---|
| `did-sdk` | 0.1.0（tag `v0.1.0`，commit `becb371`） | git 依赖 |
| `keyvault-core` | 0.1.0（同一 tag） | git 依赖 |
| `subxt` | 0.50.3 | MSRV 1.88；仅本 crate 依赖 |
| `reqwest` | 0.12.28 | `default-features = false, rustls-tls` |
| `tokio-tungstenite` | 0.24.0 | `rustls-tls-webpki-roots` |
| `rustls` / `ring` | 0.23.46 / 0.17.14 | `ring` 为 rustls 传递依赖（D-06） |
| `tokio` | 1.54.1 | |
| `wiremock` / `temp-env` | 0.6.5 / 0.3.6 | 测试用 |
| 依赖总数 | 472（lock 全量） | `cargo tree` 中无 `openssl` / `native-tls` |

> `secrecy = "0.10"` 不再声明 `features = ["zeroize"]`（该 feature 在 0.10 不存在，
> 与 did-sdk 的 D-02 结论一致）。

### 2.4 已完成的验证

| 检查 | 结果 |
|---|---|
| `git init -b main` | 见 §5 P12（初始化提交后执行） |
| `cargo generate-lockfile` | 完成（`Cargo.lock`，472 个包锁定） |
| `cargo check --locked --all-targets --all-features` | **通过**（1.88.0，44.7s） |
| `cargo fmt --all -- --check` | **通过** |
| `cargo clippy --locked --all-targets --all-features -- -D warnings` | **通过**（零告警） |
| `cargo test --locked --all-features` | **通过**（0 tests，骨架占位） |
| `cargo run --example mini_cli` | **通过**（打印版本与配置根） |
| `cargo tree -p did-sdk` 含 `subxt` | **0 处**（R1 边界成立） |
| 依赖树含 `openssl` / `native-tls` | **无** |

### 2.5 环境限制与应对

| 限制 | 影响 | 应对 |
|---|---|---|
| 沙箱只允许写 workspace | `~/.cargo` 不可写 | 本地 `CARGO_HOME=$PWD/.cargo-home`（gitignored） |
| 沙箱 `~/.rustup` 不可写 | 不能装 1.88 | 复用本机已有的 1.88.0（`RUSTUP_HOME` 指向可用目录）；CI 直接安装 |
| libgit2 SSL 握手失败 | git 依赖拉取失败 | `.cargo/config.toml` 设 `net.git-fetch-with-cli = true`（gitignored） |
| 全局 crates 镜像为 git index 且 SSL 失败 | 依赖拉取失败 | gitignored `.cargo/config.toml` 指向 Tuna **sparse** 镜像 |
| 无 `gh` / 无 PAT / SSH key 未授权 | **无法创建或推送 GitHub 仓库** | 见 D-18：需用户创建空仓库并提供凭据 |
| 无 zig / cargo-zigbuild | 本地不能交叉编译 | 跨平台只在 GitHub Actions runner 执行（A5/A6） |
| 无 identity-chain 元数据 | Chain 模式不能真实验收 | 动态运行时元数据 + `#[ignore]` 真实链测试（D-06） |

---

## 3. 架构与模块边界

### 3.1 crate 依赖图

```text
chain-owner-sdk（外部；治理）
        │
        ▼
chain-sdk ──> did-sdk ──> keyvault-core
    │              │            ▲
    │              └────────────┘
    └──> subxt（仅本 crate）

注：did-sdk 不依赖 subxt（R1）；chain-owner-sdk 复用本 crate 的读能力与 keys::sign_extrinsic。
```

### 3.2 运行时对象图

```text
Arc<dyn ChainSdk>
  ├── MockChainSdk        { core, state: async_lock::RwLock<MockState>, subs, failures }
  ├── CentralizedChainSdk { core, http: reqwest::Client, cache, subs, ws }
  └── ChainModeSdk        { core, client: OnceCell<OnlineClient>, cache, subs }

SdkCore {
    vault: Option<Arc<dyn KeyVault>>,       // Issuer: Some
    did:   Option<Arc<dyn DidPort>>,        // 两角色都装配（D-02）
    config: ChainSdkConfig,
    role: SdkRole,
    keys: Option<LoadedKeys>,               // Issuer: Some
}
```

`SdkCore` 收敛三后端共享逻辑：角色门禁、两类密钥签名、did 适配、错误映射、
`subscribe_event` 回调表。

### 3.3 关键数据流

| 流程 | 步骤 |
|---|---|
| 构造 | `resolve_config` → `build_vault`（仅 Issuer）→ `build_did_sdk`（两角色）→ `load_keys`（前缀校验，仅 Issuer）→ 选择后端 |
| 锚定写 | `ensure_issuer` → 后端写 → `emit(AnchorCreated)` → 持久化（Mock） |
| 自身管理写 | `ensure_self(pid)`（内部先 `ensure_issuer`）→ 校验链上记录 → 更新 → `emit(ParticipantUpdated)` |
| 断言签名 | `ensure_signer` → `keys::sign_participant_assertion_inner`（强制 `participant-did/`）→ `Ed25519Signature` |
| HTTP 写 | `ensure_issuer` → canonical bytes → `keys::sign_extrinsic`（强制 `chain-account/`）→ `x-chain-*` 头 → POST |
| 链写（POC） | `ensure_issuer` → `Unsupported`（D-06） |
| 事件 | 订阅表 `std::sync::RwLock<Vec<Sub>>`；emit 时短锁、不跨 `.await` |
| 错误 | `KeyVaultError` → `map_vault_err` → `ChainSdkError`（唯一路径） |

### 3.4 存储布局

```text
{CHAIN_SDK_HOME}/
├── participant-keys/                  # FileKeystore（Issuer）
│   ├── index.json
│   └── chain-account/ participant-did/
├── participant-keyvault.password      # 自动生成时 0600
├── participant-vc-store/              # did-sdk 管理（chain-sdk 不读写）
│   ├── .vc_meta
│   └── .vc_password
└── mock-state.json                    # Mock 持久化（可选）
```

所有写入采用**同目录临时文件 + fsync + rename**（与 did-sdk 一致）。

### 3.5 错误映射（唯一路径）

| `KeyVaultError` | `ChainSdkError` |
|---|---|
| `KeyNotFound` | `SignerUnavailable("key not found: ...")` |
| `KeyAlreadyExists` | `InvalidArgument("key exists: ...")` |
| `UserRejected` | **`UserRejected`（1:1）** |
| `InvalidPassword` / `Locked` | `SignerUnavailable(...)` |
| `SigningFailed` | `SigningFailed` |
| `IoError` | `ChainUnavailable` |
| `InvalidKeyFormat` / `SerializationError` | `InvalidArgument` |
| `DecryptionFailed` | `SigningFailed("decryption failed")` |
| `Unsupported` | `InvalidArgument("unsupported keystore operation")` |
| `NamespaceViolation`（规格未列） | **`KeyNamespaceViolation`** |
| `AlgorithmMismatch`（规格未列） | `InvalidArgument` |

### 3.6 三后端语义差异

| 能力 | Mock | Centralized | Chain |
|---|---|---|---|
| 锚定读 | 内存 map | `GET /anchors/{id}` + TTL | subxt dynamic storage |
| 锚定写 | 内存写 + 持久化 | `POST` + `x-chain-*` 签名 | **`Unsupported`**（D-06） |
| 参与方读 | 内存双向索引 | `GET /participants...` | subxt storage |
| 自身管理写 | 内存写 | `POST /participants/{id}/...` | **`Unsupported`** |
| 断言签名 | 本地 vault | 本地 vault（不走 HTTP） | 本地 vault |
| 事件 | 内存 emit | WebSocket `/events` | subxt `events().subscribe()` |
| 故障注入 | 支持（S5） | — | — |
| 角色拦截 | 一致 | 一致 | 一致（先于任何链调用） |

### 3.7 规格偏离矩阵（预览，详见 §4）

| 规格一位置 | 字面要求 | 本方案 | 决策 |
|---|---|---|---|
| §2 | Rust 1.85 | 1.88（did-sdk/subxt 硬要求） | D-01 |
| §4.1 | `Option<Arc<dyn did_sdk::DidSdk>>` | did-sdk 无可对象化 trait → 内部 `DidPort` | D-02 |
| §4.7 | `did_sdk::config::ConfigOverrides` 字段 | did-sdk 实际字段（`mode`/`vc_storage_path`/…）且 Server 无 `resolve_did_key` | D-02 |
| §4.8 | `ChainSdkConfig` 扁平字段 + TOML 嵌套段 | `FileConfig` 解析嵌套段并合并 | D-03 |
| §4.8 | 未定义 VC 存储路径 | 新增 `[chain_sdk.did] vc_storage_path` | D-03 |
| §4.9 | `with_preloaded_state` | 新增 `[chain_sdk.mock] state_path` | D-03 |
| §3.2 | `KeyVaultKeys` 示例 `chain-account/operator-1` | keyvault-core 按公钥生成 KeyId，无 import API | D-04 |
| §4.11 | Chain 写 POC 允许 `Unsupported` | 本期仅实现读写能力中的读 + 角色拦截；写 `Unsupported` | D-06 |
| §4.10 | `op || \n || canonical_json(body)` | 定义 canonical JSON（键排序）与错误码映射 | D-07 |
| §4.6 | `map_vault_err` 11 个变体 | 实际 13 个变体 | D-09 |

---

## 4. 关键决策（评审重点）

> **批准方式**：回复「D-01…D-20 全部批准」，或逐条给出修改。任何一条被否决，
> 先更新本文件与相关契约，再进入对应阶段。

### D-01　MSRV 提升到 1.88.0（+ `resolver = "3"`）

- **冲突**：规格一要求 `rust-version = "1.85"`；`did-sdk v0.1.0` 的
  `rust-version = "1.88"`，`subxt 0.50.3` 的 MSRV 也是 1.88。
- **建议**：`1.88.0` + `resolver = "3"` + 提交 `Cargo.lock`。
- **理由**：1.85 下 `cargo check` 直接失败（依赖 MSRV 门禁）；1.88 是当前生态的
  最小可行版本。不改公开 API。
- **影响**：`rust-toolchain.toml`、CI、README 全部固定 1.88.0。

### D-02　did-sdk 适配：内部 `DidPort`，不持有 `dyn DidSdk`

- **冲突**：规格一写 `Option<Arc<dyn did_sdk::DidSdk>>`；但 `did-sdk 0.1.0` 的
  `DidSdk<M: SdkMode>` 是**具体泛型结构**，不是 trait，且能力按 `Client`/`Server`
  分别 impl：
  - `resolve_did_key` / `resolve_did_peer` 仅在 `DidSdk<Client>`；
  - `resolve_did_web` / `assemble_vc_with_proof` / `verify_vc` 仅在 `DidSdk<Server>`；
  - `resolve_did` 是**同步**方法且走 did-sdk 自己的后端。
- **建议**：
  1. `SdkCore.did: Option<Arc<dyn DidPort>>`；`DidPort` 是 `chain-sdk` 内部 trait
     （`resolve_did_key` / `resolve_did_web` / `assemble_vc` 由 P11 决定是否加入）；
  2. `DidSdkAdapter` 持有 `Arc<did_sdk::DidSdk<did_sdk::Server>>`，以
     `key_vault = None` 构造（签名私钥只在 chain-sdk，S14/S17）；
  3. `resolve_did_key` 用 `keyvault_core::crypto::decode_did_key_pubkey`（离线、
     自包含），**不依赖** did-sdk 的 Client 模式；
  4. `resolve_did`（链上记录）由 `chain-sdk` 自己的三后端实现，**不**委托 did-sdk；
  5. `resolve_did_web` 委托 `DidSdk<Server>::resolve_did_web`（HTTPS）。
- **理由**：保持“按角色装配 did-sdk”的意图，同时让内部可注入 mock（测试），
  避免把 `Arc<DidSdk<Server>>` 的具体类型泄漏到后端实现里。

### D-03　配置 schema：嵌套 TOML + 新增 `did` / `mock` 段

- **冲突**：规格一的 `ChainSdkConfig` 是扁平字段（`centralized_endpoint` /
  `chain_endpoint`），但 TOML 样例用 `[chain_sdk.centralized]` / `[chain_sdk.chain]`
  嵌套段；且未定义 `did-sdk` 的 VC 存储路径、Mock 持久化路径。
- **建议**：
  - 公开结构体保持扁平（规格一 API 不变）；
  - `resolve_config` 内部用 `FileConfig`（全 `Option`）解析嵌套段后映射；
  - 新增两段：
    - `[chain_sdk.did] vc_storage_path`（`ChainSdkConfig.did: DidConfig`）；
    - `[chain_sdk.mock] state_path`（`ChainSdkConfig.mock: MockConfig`，`Option`）；
  - 对应环境变量：`CHAIN_SDK_DID_VC_STORAGE_PATH`、`CHAIN_SDK_MOCK_STATE_PATH`。
- **理由**：`did-sdk<Server>` 构造必须提供 VC 存储目录；Mock 持久化必须有落点。
  两者都是实现必需项，规格一遗漏。

### D-04　密钥 provisioning：配置 `KeyId` 必须匹配已存在的 key

- **冲突**：规格一样例 `operator = "chain-account/operator-1"`；但
  `keyvault-core` 的 `generate(namespace, algorithm)` **不接收自定义 id**，生成的
  `KeyId` 是 `chain-account/<SS58>` / `participant-did/did_key_<multibase>`，
  且没有 `import` API。
- **建议**：
  - 配置中的 `KeyId` 视为**声明**：`sign_*` 时由 `keyvault-core` 查找，缺失返回
    `SignerUnavailable`（经 `map_vault_err`）；
  - **不在生产路径静默生成密钥**（避免静默创建身份）；
  - 提供 `examples/mini_cli init-keys`（P10）与测试辅助 `seed_keys`：生成后**打印
    实际 `KeyId`**，由运维写入配置；
  - `health_check`（Issuer）做一次 `vault.load` 检查并报告缺失 key。
- **理由**：密钥应由治理/运维 provisioning；自动生成会让 `controller_did` 与
  `operator` 的绑定关系失去可控性。
- **影响**：`tests/configs/*.toml` 中的示例 id 仅用于配置解析测试；签名测试使用
  `MemoryKeystore` 生成的真实 `KeyId`。

### D-05　`did-sdk<Server>` 的构造与副作用

- **建议**：两角色都构造 `DidSdk<Server>`（S17 要求 Verifier 也有 `did-resolution`）：
  - `participant_did = config.participant.controller_did`；
  - `key_vault = None`（签名私钥只在 chain-sdk）；
  - `mode`：`Mock` → did-sdk `Mock`；`Centralized`/`Chain` → did-sdk `Centralized`；
  - `vc_storage_path = config.did.vc_storage_path`。
- **已知副作用**：`VcStore::new` 会创建目录与 `.vc_meta` / `.vc_password`，即使
  `vc_wallet = false`。这是 did-sdk 的存储初始化行为，README 已注明。
- **理由**：S17 的 `vc-wallet` 对 Verifier 是“可选”；可选开启时必须有存储路径。

### D-06　Chain 后端 POC 范围：动态元数据读 + 角色拦截；写 `Unsupported`

- **现状**：无 `metadata/identity_chain.scale`；`subxt 0.50` 可在运行时拉取元数据。
- **建议**：
  1. 读：`OnlineClient::from_url` + `subxt::dynamic::storage`，pallet / storage 项名
     通过环境变量覆盖（默认 `Anchor` / `Anchors`，`Participant` / `Participants`）；
  2. 写：本期返回 `Unsupported`（规格一 P11 允许）；**角色门禁先执行**，保证
     S9/S17/S19 在 chain 模式通过；
  3. 事件：`client.events().subscribe()` 订阅并按 pallet/event 名过滤；
  4. 真实链测试 `#[ignore]`，需 `CHAIN_SDK_CHAIN_ENDPOINT`。
- **为什么不实现 extrinsic 提交**：`subxt::tx::Signer::sign` 的签名是
  `fn sign(&self, payload: &[u8]) -> T::Signature`，**没有 `Result`**，而
  `KeyVault::sign` 是 async 且可失败（含 `UserRejected`）。用同步桥 + 副作用标志
  可以“绕”，但会把 `UserRejected` 降级为 panic/零签名，安全性不可接受。
  待 subxt 提供 fallible signer（或我们改用预签名 payload）后再实现。
- **理由**：诚实标注能力边界，避免“能编译但运行必错”的假实现。

### D-07　Centralized 契约：端点、canonical JSON、路径编码

- **建议**（附录 C 为完整契约）：
  - 读：`GET /health`、`GET /anchors/{id}`、`GET /anchors?anchor_type=&submitter_id=`、
    `GET /participants`、`GET /participants/{id}`、`GET /participants/by-did/{did}`、
    `GET /dids/{did}`；
  - 写：`POST /anchors`、`POST /anchors/{id}/revoke`、`POST /anchors/{id}/renew`、
    `POST /participants/{id}/domains`、`POST /participants/{id}/controller-did`；
  - 签名：`bytes = op || "\n" || canonical_json(body)`；
    头：`x-chain-signer`（`KeyId`）、`x-chain-signature`（hex）、`x-chain-op`；
  - `canonical_json`：对象键递归升序、无空白、拒绝浮点（附录 C.3）；
  - 路径参数用 `reqwest::Url::path_segments_mut` 编码（DID 含 `:` / `/`）。
- **理由**：规格一给了端点映射但没给 canonical 定义与编码规则；不定义会导致
  Mock 服务与 SDK 的签名不一致。
- **待确认**：中心化模拟链服务是否已有实现（Q-01）；若已有，以对方契约为准。

### D-08　事件订阅：三后端统一 `EventData`，Centralized 用 WebSocket

- **建议**：
  - 回调表：`std::sync::RwLock<Vec<Sub { id, event_name, callback: Arc<...> }>>` +
    `AtomicU64`；`emit` 时**先收集再释放锁**，不跨 `.await`；
  - Mock：写操作后同步 emit；S11 测试用 `tokio::sync::Notify` 等待；
  - Centralized：`GET {endpoint}/events`（WebSocket，JSON `EventData` 帧）；
    断线指数退避重连；最后一个订阅取消时关闭连接；
  - Chain：`subxt` 事件流按 pallet/event 名过滤（P10）；无节点时 `Unsupported`。
- **待确认**：事件名清单（Q-05）；WS 帧是否带 envelope（建议裸 `EventData`）。

### D-09　`map_vault_err` 覆盖 `keyvault-core` 全部 13 个变体

规格一只列 11 个；实际 `KeyVaultError` 还有 `NamespaceViolation` 与
`AlgorithmMismatch`。映射见 §3.5，`NamespaceViolation → KeyNamespaceViolation`。

### D-10　`Unsubscribe` 与回调生命周期

- `Unsubscribe = Box<dyn FnOnce() + Send + Sync>`；
- 返回的闭包捕获 `Arc<SubscriptionTable>` + `id`，调用时移除；
- 重复调用 / 订阅已取消：幂等（不 panic）；
- `subscribe_event` 是同步方法，不获取 async 锁。

### D-11　测试与 lint 策略

- 所有 public 项写 doc（`missing_docs = warn` + CI `-D warnings`）；
- 测试模块顶部 `#![allow(clippy::unwrap_used, clippy::expect_used)]`；
- 集成测试通过 `tests/common/mod.rs` 复用构造器；
- 环境变量测试用 `temp-env`（Rust 2024 `set_var` 是 `unsafe`，业务代码
  `unsafe_code = "forbid"`）；
- `wiremock` 用于 Centralized；WebSocket 测试起本地 `tokio-tungstenite` 服务。

### D-12　S17 的验证方式：行为断言 + 内部单测

规格一的 `tests/capabilities.rs` 想断言 `core.did.is_some()` / `core.vault.is_none()`，
但 `SdkCore` 是 `pub(crate)`，集成测试不可见。建议：

- **集成测试（公开行为）**：
  - Verifier：`resolve_did_key` 可用（did-resolution 生效）；
  - Verifier：`sign_participant_assertion` → `SignerUnavailable`（无签名器）；
  - Verifier：`create_anchor` → `Unauthorized`（无写能力）；
  - Issuer：两类签名可用（`sign_extrinsic` 效果通过 Centralized 签名头验证）；
- **单元测试（内部字段）**：在 `config.rs` 的 `#[cfg(test)] mod tests` 中断言
  `core.did.is_some()` / `core.vault.is_none()` / `core.keys` 的装配矩阵。
- **理由**：不为了测试暴露 `#[doc(hidden)]` 内部访问器，保持公开 API 干净。

### D-13　SD-JWT 的 API 归属（评审重点）

- **方案 A（推荐）**：`ChainSdk` 增加 supertrait `SelectiveDisclosure`
  （`issue_sd_credential` / `verify_sd_presentation`），三后端统一实现；
  方法在未启用 `sd-jwt` feature 时返回 `Unsupported`。
  - 优点：`Arc<dyn ChainSdk>` 直接可用（Rust 1.86+ 支持 trait upcasting）；
  - 缺点：`ChainSdk` trait 增加 2 个方法（S1 的“16 方法”变成 18）。
- **方案 B**：独立 trait + 独立工厂 `build_sd_service(&ChainSdkConfig)`；
  `ChainSdk` 不变。缺点：需要额外装配入口，且验证需要复用 `get_anchor`。
- **方案 C**：独立 crate `chain-sdk-sd`。缺点：本期不发布 crates.io，过度拆分。
- **建议**：A，但**仅在 D-14/D-15/D-16 批准后**进入 P11；否则先交付 P0–P10。

### D-14　SD-JWT 哈希公式：按附件实现 `sd-v1`

- **冲突**：SD-JWT 附件定义
  `H = SHA-256(base64url(salt) || base64url(name) || base64url(value))`，
  而 RFC 9901 的 digest 是对 **base64url(JSON disclosure)** 求 SHA-256。
- **建议**：按附件实现，标记为 `sd-v1`；`_sd_alg = "sha-256"`；
  `_sd` 元素 = `base64url_nopad(digest)`；并在文档与测试向量中锁定。
  - 若评审选择 RFC 9901，需同步修改附件的 I1/I2/V3 与本文档附录 D（工作量相近）。
- **理由**：附件是本次验收依据；偏离需先改验收标准。
- **风险**：与外部 SD-JWT 实现不互通（POC 可接受）。

### D-15　`payload_hash` 算法：沿用锚定层 `blake2b-256`

- **冲突**：SD-JWT 附件写 `payload_hash = SHA-256(canonicalized VC JSON)`；
  但现有锚定约定（`tests/fixtures` 的 `"blake2b:..."`、did-sdk 的 `vc_hash`）使用
  **BLAKE2b-256**。
- **建议**：
  - `payload_hash` = `blake2b:` + hex(`BLAKE2b-256(compact_json(VC without proof))`)，
    与 did-sdk 的 `vc_hash_without_proof` 一致；
  - SD-JWT 的 `_sd` 哈希用 **SHA-256**（与 `_sd_alg` 一致），两者互不影响。
- **理由**：链上锚定与网关校验必须一致；单方面改成 SHA-256 会让既有锚定失配。
- **待确认**：Q-03（是否需要 RFC 8785 JCS）。

### D-16　did-sdk 模型缺口与跨仓库依赖（评审重点）

SD-JWT 附件要求 VC 顶层 `credentialStatus.anchorId`、VP 顶层 `disclosures`，
且持有者签名覆盖 disclosures。`did-sdk 0.1.0` 现状：

| 需求 | did-sdk 0.1.0 现状 |
|---|---|
| VC 顶层 `credentialStatus` | **无**；只有 `anchor_id: Option<String>`（顶层 `anchor_id`） |
| VP 顶层 `disclosures` | **无** |
| VP 签名输入覆盖 disclosures | **无**；`vp_v1` 只覆盖 holder/challenge/domain/VC 哈希 |
| VC 签发时注入外部签名 | `assemble_vc_with_proof` 需要**先有 proof**（鸡生蛋）；`vc_signing_input` 是 `pub(crate)` |
| VC 存储保留未知字段 | 序列化为 `VerifiableCredential`，**丢弃** `credentialStatus` |

- **结论**：
  1. `chain-sdk` 的 SD 模块必须定义**自包含**的 `SdCredential` / `SdPresentation`
     JSON 契约（`serde_json::Value` 承载 VC，保留全部字段），不依赖 did-sdk 的
     VC/VP 结构做端到端；
  2. VC 签名验证在 chain-sdk 内用 `keyvault_core` 的 Ed25519 原语 +
     与 did-sdk 一致的 `IssuedVc-v1` 签名输入（`issued_vc_v1` 是 pub 函数）；
  3. **端到端交付（I6 / D1–D5）依赖 did-sdk v0.2**：增加 `credentialStatus`、
     `disclosures`、`VP-v2` 签名输入、两阶段 VC 组装（`build_vc` + `sign` +
     `attach_proof`）与未知字段保留。该改动**不在本仓库**，列为外部依赖。
- **建议**：P11 只交付 chain-sdk 侧的签发/验证原语 + 自包含 JSON 契约 + 测试；
  did-sdk 侧改动另行评审，避免把两个仓库的破坏性变更捆在一次提交里。
- **待确认**：Q-06（是否本期交付 P11）。

### D-17　版本号：`0.1.0`

规格一写 `version = "0.2.0"`（延续旧版），但本仓库无公开历史，且 `did-sdk` 为
`0.1.0`。建议首个版本 `0.1.0`；若评审要求与规格一一致，改为 `0.2.0`（一处改动）。

### D-18　GitHub 仓库与凭据（**当前唯一外部阻塞项**）

- 现状：`metercai/chain-sdk` **不存在**（API 404）；本机**无 `gh` CLI**、
  **无 PAT**、`~/.ssh/id_rsa` **未被 GitHub 账户授权**（`Permission denied (publickey)`）。
- **需要用户二选一**：
  - **(a)** 在 GitHub 网页创建空仓库 `metercai/chain-sdk`（不要初始化 README），
    并在本机配置 HTTPS 凭据（PAT）或把本机公钥加入账户；或
  - **(b)** 提供一个有 `repo` scope 的 PAT（仅本次推送使用，不写入仓库/日志），
    由我创建仓库并推送。
- 推送后：`origin` = HTTPS，`main` 分支，4 个 workflow 自动运行。
- **在此之前**：所有工作只在本地完成，不伪造远程状态。

### D-19　本地沙箱工具链与镜像（不提交）

- `CARGO_HOME=<repo>/.cargo-home`、`RUSTUP_HOME` 指向可用 1.88.0；
- `.cargo/config.toml`：Tuna sparse 镜像 + `net.git-fetch-with-cli = true`；
- 以上均 gitignored；CI 使用默认 crates.io，无需这些文件。

### D-20　不提供绑定；未来扩展路径

- 不提供 Python / Node / WASM 绑定；
- `device-keystore` / `server-keystore` 只保留 feature 与 `build_vault` 分支；
- BBS+ 作为 SD-JWT 的并存机制（`_bbs` 字段），不在本期。

---

## 5. 分阶段实施步骤

### 总体节奏

| 阶段 | 主题 | 预计 | 门禁 |
|---|---|---|---|
| P0 | 契约落地 + 配置解析 + 工厂骨架 | 1.5 天 | A1/A2 + `config_resolution` |
| P1 | Mock 读 + 持久化 + 故障注入 | 1.5 天 | `mock_persistence` / `mock_fault_injection` |
| P2 | Mock 写 + 角色/自身拦截 | 1 天 | `role_gate` |
| P3 | 两类密钥 + namespace（S14） | 0.5 天 | `keys_namespace` |
| P4 | did_adapter 按角色装配（S17） | 1 天 | `capabilities` |
| P5 | Centralized 读 + TTL 缓存 | 1 天 | `centralized_contract` / `centralized_cache` |
| P6 | Centralized 写 + 签名（S7） | 1 天 | `centralized_signing` |
| P7 | 参与方绑定 + 委托锚定（S15/S16/S20） | 0.5 天 | `participant_binding` |
| P8 | 无治理接口 + 契约检查（S18） | 0.5 天 | `no_governance` + `verify-roles.sh` |
| P9 | 事件订阅三后端（S11） | 1 天 | `event_subscription` |
| P10 | Chain 后端（subxt 读 + 事件）+ mini_cli | 2 天 | `role_matrix`（chain）+ CI |
| P11 | 一致性与全量验收（S3/S12） | 1 天 | A3/A4 |
| P12 | SD-JWT 原语（若 D-13～D-16 批准） | 2 天 | `sd_*` |
| P13 | 文档、GitHub 推送、CI 全绿 | 0.5 天 | A11 |

每个阶段结束必须：跑门禁命令 → 更新 `docs/acceptance-checklist.md` 证据 →
`git commit`（`P<N>: ...`）→ 才能进入下一阶段。

---

### P0 — 契约落地 + 配置解析 + 工厂骨架

**目标**：`resolve_config` / `build_chain_sdk` 的装配路径可用；后端未实现处返回
`Unsupported`。

**任务**

1. `src/role_gate.rs`：`RoleGate` trait（`ensure_issuer` / `ensure_self` / `ensure_signer`）。
2. `src/keys.rs`：
   - `CHAIN_ACCOUNT_PREFIX` / `PARTICIPANT_DID_PREFIX`；
   - `LoadedKeys`、`load_keys`（前缀校验，S14）；
   - `sign_extrinsic` / `sign_participant_assertion_inner`（唯一入口，前缀校验）。
3. `src/config.rs`：
   - `FileConfig`（`[chain_sdk]` + `key_vault` + `participant` + `did` + `mock` +
     `capabilities` + `centralized` + `chain`）；
   - `resolve_config`：overrides > env > toml > default；
   - `build_vault`：`file-keystore` / `memory-keystore`；仅 Issuer 解析密码；
   - `build_chain_sdk`：装配 vault（Issuer）/ did（两角色）/ keys（Issuer），
     后端构造留占位。
4. `src/events.rs`：`SubscriptionTable` 骨架（P9 完成）。
5. `src/mock.rs` / `centralized.rs` / `chain.rs`：模块与类型骨架，方法返回
   `Unsupported("P<N>")`。
6. `tests/config_resolution.rs`：
   - 默认值；env 覆盖；override 优先；
   - `role = "app"` → `Issuer`；
   - Verifier 不解析密码（`resolved_password.is_none()`）；
   - `tests/configs/*.toml` 可解析；
   - 非法 mode/role 报 `ConfigError`。

**门禁**

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --test config_resolution --test keys_namespace
```

**完成标准**：S1 / S3-AppAlias 通过；`build_chain_sdk` 对三种 mode 都能返回
`Arc<dyn ChainSdk>`（后端方法 `Unsupported`）。

---

### P1 — Mock 读 + 持久化 + 故障注入

1. `MockState`（anchors / participants / 双向索引 / registered_anchor_types / dids /
   current_time / next_anchor_seq）；
2. `MockChainSdk::new` / `with_preloaded_state(path)` / `with_state_path`；
3. 读方法：`health_check`（`healthy=true`, `mode="mock"`）、`get_anchor`、
   `list_anchors`（过滤）、`get_participant`、`get_participant_by_did`、
   `list_participants`、`resolve_did`、`resolve_did_key`、`resolve_did_web`；
4. 持久化：写后原子写 JSON；`with_preloaded_state` 读取 `tests/fixtures/mock_state.json`；
5. 故障注入：`FailureRule::{Latency, Unavailable, Specific}`、
   `add_failure` / `set_chain_unavailable` / `advance_time`；
6. 测试：`mock_persistence`（预置可读、写后更新、重启后仍在）、
   `mock_fault_injection`（延迟、不可用、指定错误）。

**门禁**：`cargo test --test mock_persistence --test mock_fault_injection`

---

### P2 — Mock 写 + 角色/自身拦截

1. `create_anchor`：`ensure_issuer` → 校验 `registered_anchor_types` →
   生成 `anchor:{seq}` → `issuer_did = controller_did`、`delegated_by_did = None`；
2. `revoke_anchor` / `renew_anchor`：`AnchorNotFound` / `AnchorRevoked` /
   `AnchorPermanent` 语义（`valid_until == 0`）；
3. `update_authorized_domains` / `update_controller_did`：`ensure_self(pid)` +
   双向索引维护；
4. `sign_participant_assertion`：`ensure_signer` → 前缀校验 → vault；
5. 测试：`role_gate`（Verifier 全部写 → `Unauthorized`；断言签名 →
   `SignerUnavailable`；Issuer 改他人 → `Unauthorized`；永久锚定续期 →
   `AnchorPermanent`）。

**门禁**：`cargo test --test role_gate`

---

### P3 — 两类密钥 + namespace（S14）

1. 完成 `keys.rs` 的 `sign_extrinsic` / `sign_participant_assertion_inner`；
2. `tests/keys_namespace.rs`：错误前缀函数入口即返回 `KeyNamespaceViolation`
   （不触达 vault，用 `MemoryKeystore` 验证）；
3. `load_keys` 前缀校验（通过 `resolve_config` + `build_chain_sdk` 触发）。

**门禁**：`cargo test --test keys_namespace`

---

### P4 — did_adapter 按角色装配（S17）

1. `DidPort` + `DidSdkAdapter`（P11 前只含 `resolve_did_key` / `resolve_did_web`）；
2. `build_did_sdk(cfg, role)`：did-sdk mode 映射、`key_vault=None`、
   `vc_storage_path`；
3. `capabilities.effective_for(role)` 在 `build_chain_sdk` 中生效；
4. `tests/capabilities.rs`：Verifier 行为断言（D-12）+ 内部单测；
5. 失败路径：did-sdk 构造失败 → `ConfigError("did-sdk: ...")`。

**门禁**：`cargo test --test capabilities`

---

### P5 — Centralized 读 + TTL 缓存

1. `CentralizedChainSdk::new`（reqwest client、timeout、user-agent、缓存）；
2. 读端点（附录 C）：`health`、`anchors`、`participants`、`dids`；
3. 错误映射：404 → `AnchorNotFound` / `ParticipantNotFound`；
   5xx → `ChainUnavailable`；连接失败 → `NetworkError`；
4. TTL 缓存：仅 `get_anchor`；`cache_ttl_secs = 0` 关闭；
5. 测试：`centralized_contract`（method + path + 查询参数 + DID 编码）、
   `centralized_cache`（命中、过期穿透、关闭缓存）。

**门禁**：`cargo test --test centralized_contract --test centralized_cache`

---

### P6 — Centralized 写 + 签名（S7）

1. `signed_request`：`ensure_issuer` → canonical bytes → `sign_extrinsic` →
   `x-chain-*` 头；
2. 写端点：`POST /anchors`、`/revoke`、`/renew`、`/participants/{id}/domains`、
   `/controller-did`；
3. `sign_participant_assertion`：本地 vault，不走 HTTP；
4. 测试：`centralized_signing`（头存在、签名可用 `MemoryKeystore` 公钥验签、
   canonical 字节稳定、Verifier 写 → `Unauthorized` 且无 HTTP 请求）。

**门禁**：`cargo test --test centralized_signing`

---

### P7 — 参与方身份绑定 + 委托锚定（S15/S16/S20）

1. fixtures：`mock_state.json` / `delegated_anchor.json` 接入；
2. `get_participant_by_did` 双向索引；`update_controller_did` 索引迁移；
3. 委托锚定：`issuer_did` = 信任根、`submitter_id` = 代理方、
   `delegated_by_did` = 代理方 DID；
4. `business_types` 不参与鉴权（篡改后行为不变）；
5. 测试：`participant_binding`。

**门禁**：`cargo test --test participant_binding`

---

### P8 — 无治理接口 + 契约检查（S18）

1. `tests/no_governance.rs`：源码扫描 + 允许方法名清单；
2. `scripts/verify-roles.sh`：治理方法 / VC 存储方法 grep；
3. CI `ci.yml` 的契约完整性步骤（16 方法存在）。

**门禁**：`cargo test --test no_governance && bash scripts/verify-roles.sh`

---

### P9 — 事件订阅三后端（S11）

1. `SubscriptionTable`（D-08/D-10）；
2. Mock：`create_anchor` / `revoke_anchor` / `update_*` 后 emit；
3. Centralized：`/events` WS 客户端 + 重连 + 取消（本地 WS 测试服务）；
4. Chain：`events().subscribe()` 过滤（P10 接线）；
5. 测试：`event_subscription`（三后端各一条；取消后不再回调；重复取消幂等）。

**门禁**：`cargo test --test event_subscription`

---

### P10 — Chain 后端 + mini_cli

1. `ChainModeSdk`：
   - `OnlineClient` 懒连接（`tokio::sync::OnceCell`）；
   - `health_check`（区块高度）；
   - 读：dynamic storage（pallet/项名 env 覆盖）；
   - 写：`ensure_issuer` / `ensure_self` / `ensure_signer` 后返回 `Unsupported`；
   - 事件：`events().subscribe()`；
2. `tests/role_matrix.rs`：3 后端 × 2 角色；chain 的 Issuer 写 → `Unsupported`
   （**不是** `Unauthorized`）；
3. `examples/mini_cli.rs`：`init-keys` / `health` / `get-anchor` / `create-anchor`；
4. 真实链集成测试 `#[ignore]`。

**门禁**：

```bash
cargo test --locked --test role_matrix
cargo build --locked --release --example mini_cli
```

**完成标准**：S9 / S17 / S19 在 chain 模式通过；本地无节点时测试用 wiremock/超时路径
覆盖；真实链验收标注「需链 fixture」。

---

### P11 — 一致性与全量验收（S3/S12）

1. `tests/consistency.rs`：同一组用例（读、写、错误码）在 Mock 与 Centralized
   （wiremock）上结果一致；
2. `scripts/run-consistency.sh`、`scripts/verify-roles.sh` 全绿；
3. `docs/acceptance-checklist.md` 逐项填证据（命令 + 结果摘要）；
4. 三平台原生构建（CI）；musl 静态（CI）。

**门禁**：A3 / A4 / A5 / A6

---

### P12 — SD-JWT 选择性披露（条件：D-13～D-16 批准）

1. `src/sd.rs`：
   - `SdDisclosureV1 { salt, name, value }`；
   - `SdCredential`（VC JSON + `credentialSubject._sd/_sd_alg` +
     `credentialStatus.anchorId`）；
   - `SdPresentation`（VC + `disclosures` + holder + challenge + domain + proof）；
   - `issue_sd_credential`：盐（16B CSPRNG）→ 哈希 → `_sd` → 组装 VC →
     锚定 → 签名 → 返回 `{vc, disclosures, anchor_id}`；
   - `verify_sd_presentation`：holder 签名 → VC 签名 → Disclosure 哈希比对 →
     `get_anchor` → payload_hash 比对（D-15）；
2. 错误码：`DisclosureHashMismatch` / `DisclosureDuplicate` /
   `DisclosureMissing` / `SdAlgUnsupported`（加入 `ChainSdkError`）；
3. 依赖：`sha2 0.10`、`base64 0.22`、`getrandom 0.4`（新增）；
4. 测试：
   - `sd_issue`：盐随机性、`_sd` 数量、Disclosures 格式（I1–I4）；
   - `sd_verify`：正常披露、未披露不可见（V1–V5）；
   - `sd_tamper`：改盐/值 → `DisclosureHashMismatch`（V6）；
   - `sd_privacy`：跨 VC 同属性哈希不同、字典攻击防护（P1/P3）；
   - `sd_duplicate`：重复披露 → `DisclosureDuplicate`（V7）；
5. 与 did-sdk 的跨仓库依赖写入 README 与验收记录（D-16）。

**门禁**：`cargo test --locked --features sd-jwt --test sd_issue --test sd_verify --test sd_tamper --test sd_privacy`（文件名以 P12 落地为准）。

---

### P13 — 文档、GitHub 推送、CI 全绿

1. 更新 README（实现状态、命令、验收结果）；
2. `docs/acceptance-checklist.md` 全量证据；
3. `git init -b main` + 初始提交 + `origin` 配置 + 推送（依赖 D-18）；
4. 等待 4 个 workflow 全绿（A11）；失败则修复后追加提交。

**门禁**：GitHub Actions `ci` / `rust-musl` / `consistency` / `roles` 全绿。

---

## 6. 测试矩阵

| 测试文件 | 覆盖 | 关键断言 |
|---|---|---|
| `config_resolution.rs` | S2 / S3-AppAlias | 默认 / env / override；`app` → `Issuer`；Verifier 不解析密码 |
| `role_gate.rs` | S9 / S19 | Verifier 写 → `Unauthorized`；断言 → `SignerUnavailable`；改他人 → `Unauthorized` |
| `role_matrix.rs` | S2 / S9 | 3×2 矩阵；chain 写 → `Unsupported` |
| `capabilities.rs` | S17 | 行为断言 + 内部装配单测（D-12） |
| `keys_namespace.rs` | S14 | 错误前缀 → `KeyNamespaceViolation` |
| `participant_binding.rs` | S15 / S16 / S20 | 双向索引；委托三字段；business_types 不参与鉴权 |
| `no_governance.rs` | S18 | 源码无治理方法 |
| `consistency.rs` | S3 / S12 | Mock vs Centralized 同结果 |
| `mock_persistence.rs` | S4 | 预置可读；写后 JSON 更新 |
| `mock_fault_injection.rs` | S5 | 延迟 / 不可用 / 指定错误 |
| `centralized_contract.rs` | S6 | method + path + 查询参数 + DID 编码 |
| `centralized_signing.rs` | S7 | `x-chain-*`；签名可验；canonical 稳定 |
| `centralized_cache.rs` | S8 | TTL 命中 / 过期 / 关闭 |
| `keyvault_integration.rs` | S10 / S13 | Issuer 加载 FileKeystore；Verifier 不加载；错误映射 |
| `event_subscription.rs` | S11 | 三后端收到事件；取消幂等 |
| `sd_issue.rs`（P12） | I1–I4 | 盐随机、`_sd` 数量、Disclosure 格式 |
| `sd_verify.rs`（P12） | V1–V5 | 披露可见、未披露不可见、锚定检查 |
| `sd_tamper.rs`（P12） | V6 / V7 | 篡改 → mismatch；重复 → duplicate |
| `sd_privacy.rs`（P12） | P1 / P3 | 跨 VC 哈希不同；不可反推 |

**测试辅助**（`tests/common/mod.rs`）：

- `build_mock(role, participant_id)` / `build_centralized(endpoint, role)`；
- `seed_keys(vault) -> (KeyId, KeyId)`（D-04）；
- `load_fixture()`；`assert_err!(result, Variant)` 宏；
- 所有测试模块 `#![allow(clippy::unwrap_used, clippy::expect_used)]`。

---

## 7. CI 与构建矩阵

| workflow | 触发 | 内容 |
|---|---|---|
| `ci.yml` | push / PR / dispatch | fmt、clippy（`-D warnings`）、`cargo test --all-features`、契约完整性 |
| `rust-musl.yml` | push / PR / dispatch | 四 target 构建 + musl 静态断言 + 产物上传 |
| `consistency.yml` | push / PR / dispatch | `consistency` + `role_gate` |
| `roles.yml` | push / PR / dispatch | 角色矩阵 + S18/S17 grep + did-sdk 无 subxt 断言 |

- 统一 `dtolnay/rust-toolchain@1.88.0`，`Swatinem/rust-cache@v2`；
- musl 用 `goto-bus-stop/setup-zig@v2`（0.13.0）+ `cargo install cargo-zigbuild`；
- 提交 `Cargo.lock`，CI 全部 `--locked`；
- 静态断言：`file` 含 `statically linked|static-pie linked`，`ldd` 无动态依赖；
- 本地不做交叉编译（D-18/§2.5）。

---

## 8. 安全与威胁模型

| 威胁 | 缓解 |
|---|---|
| 角色越权写 | `ensure_issuer` / `ensure_self` 先于所有后端逻辑；三后端一致 |
| 密钥混用（sr25519 当 ed25519 用） | 两个独立签名函数 + 类型级隔离 + 前缀校验 + keyvault-core 内部校验 |
| 私钥泄露 | `KeyVault` 不导出私钥；`SecretString` redacted Debug；日志不打印 key 内容 |
| 配置注入 | 环境变量仅覆盖白名单字段；非法值 → `ConfigError` |
| 重放（Centralized 写） | 签名覆盖 `op + canonical body`；建议服务端加 nonce/时间窗（Q-07） |
| 用户拒绝签名被吞 | `KeyVaultError::UserRejected` → `ChainSdkError::UserRejected` 1:1 |
| Mock 被误用于生产 | `mode` 显式配置；`health_check.mode` 暴露；README 标注 |
| SD-JWT 隐私边界误读 | 文档明确：不防同一 VC 跨出示关联、不防验证方合谋拼接（SD-JWT 附件 §5） |
| 选择性披露篡改 | 发行方签名覆盖 `_sd`；持有者签名覆盖 disclosures；逐条重算哈希 |
| 供应链 | `Cargo.lock` + `--locked`；禁止 openSSL/native-tls；`audit` 另行接入 |

---

## 9. 依赖约束与平台说明

1. **musl 静态**：`reqwest(rustls-tls)` + `tokio-tungstenite(rustls-tls-webpki-roots)`；
   禁止 `native-tls` / `openssl`；`ring` 可接受（纯 Rust + asm）。
2. **subxt 仅在本 crate**：`did-sdk` 依赖树中不得出现 `subxt`（R1），CI 断言。
3. **`did-sdk` / `keyvault-core` 不在本仓库**：不建子目录、不复制 trait/实现。
4. **feature 门控**：`did-resolution` / `vc-wallet` / `issuer-signing` /
   `sd-jwt` / `device-keystore` / `server-keystore`；默认全关，能力由配置决定。
5. **Windows**：路径用 `PathBuf`；文件名不使用 DID 原文（`did:web:...` 含冒号），
   VC 存储文件名由 did-sdk 负责（哈希）。
6. **时间**：内部 `u64` Unix 秒；不引入 `chrono`/`time`（MSRV 与体积）。

---

## 10. 风险登记

| ID | 风险 | 概率 | 影响 | 处置 |
|---|---|---|---|---|
| R-01 | Chain 后端无真实元数据，dynamic storage 键构造错误 | 高 | 中 | P10 用 `#[ignore]` 真实链测试；pallet/项名 env 覆盖；文档标注 |
| R-02 | `subxt::Signer` 无 `Result`，写路径无法透传 `UserRejected` | 高 | 中 | 本期写 `Unsupported`（D-06）；后续评估预签名 payload |
| R-03 | Centralized 服务契约未定（Q-01） | 中 | 高 | 附录 C 作为提议契约；对接前先评审 |
| R-04 | canonical JSON 与中心化服务不一致 | 中 | 高 | 锁定附录 C.3 + 共享测试向量；建议服务端复用同一实现 |
| R-05 | SD-JWT 哈希公式与 RFC 9901 不兼容 | 高 | 中 | D-14 明确 `sd-v1`；文档标注不与外部互通 |
| R-06 | did-sdk 模型缺口导致 SD 端到端不可交付 | 高 | 高 | D-16：本期只做原语；did-sdk v0.2 另立任务 |
| R-07 | `KeyVaultKeys` 示例 id 与 keyvault 生成 id 不一致 | 高 | 中 | D-04：init-keys 打印真实 id；health_check 报告缺失 |
| R-08 | GitHub 凭据缺失，无法推送/跑 CI | 高 | 高 | D-18：用户创建仓库并提供 PAT/SSH |
| R-09 | musl 静态被 `ring`/`rustls-platform-verifier` 破坏 | 低 | 高 | CI 静态断言；必要时禁用 platform-verifier |
| R-10 | Windows 路径/换行差异 | 低 | 中 | 原子写 + `PathBuf`；CI windows 构建 |
| R-11 | Clippy `-D warnings` 与 `missing_docs` 阻碍合并 | 中 | 低 | 所有 public 项写 doc；测试局部 allow |
| R-12 | 事件 WS 断线/乱序 | 中 | 中 | 重连 + 单调 `block_number`；测试覆盖断线 |

---

## 11. 评审确认清单

请逐条确认（可回复「全部批准」或给出修改）：

| 编号 | 决策 | 建议 |
|---|---|---|
| D-01 | MSRV 1.88.0 + `resolver = "3"` | 批准 |
| D-02 | did-sdk 适配为内部 `DidPort`，`Arc<DidSdk<Server>>` | 批准 |
| D-03 | 嵌套 TOML + 新增 `did` / `mock` 配置段 | 批准 |
| D-04 | 密钥 provisioning（不静默生成） | 批准 |
| D-05 | 两角色都构造 `DidSdk<Server>`，`key_vault=None` | 批准 |
| D-06 | Chain POC：读 + 事件 + 角色拦截；写 `Unsupported` | 批准 |
| D-07 | Centralized 契约 + canonical JSON | 批准 |
| D-08 | 事件订阅协议（WS / subxt events） | 批准 |
| D-09 | 错误映射补全 13 变体 | 批准 |
| D-10 | `Unsubscribe` 幂等设计 | 批准 |
| D-11 | 测试与 lint 策略 | 批准 |
| D-12 | S17 行为断言 + 内部单测 | 批准 |
| D-13 | SD-JWT API 归属：方案 A（supertrait） | 需拍板 |
| D-14 | SD-JWT 按附件 `sd-v1` 公式 | 需拍板 |
| D-15 | `payload_hash` 沿用 blake2b-256 | 需拍板 |
| D-16 | did-sdk 模型缺口：本期只交付原语 | 需拍板 |
| D-17 | 版本号 0.1.0 | 建议 |
| D-18 | GitHub 仓库/凭据（外部阻塞） | 需用户操作 |
| D-19 | 本地沙箱工具链不提交 | 批准 |
| D-20 | 不提供绑定；未来扩展路径 | 批准 |

### 待确认问题

| 编号 | 问题 | 影响 |
|---|---|---|
| Q-01 | 中心化模拟链服务是否已有实现/规格？端点与错误码以哪份为准？ | P5/P6 |
| Q-02 | identity-chain 的 pallet / storage 项名能否提供？ | P10 |
| Q-03 | canonical JSON 是否需要 RFC 8785（JCS）？ | P6 / P12 |
| Q-04 | 链上 `chain_role` 与 `SdkRole` 不一致时是否启动报错（严格模式）？ | P0/P2 |
| Q-05 | 事件名清单（`AnchorCreated` / `ParticipantUpdated` / ...）？ | P9 |
| Q-06 | SD-JWT（P12）是否本期交付？若交付，是否同时排期 did-sdk v0.2？ | P12 |
| Q-07 | Centralized 写请求是否需要 nonce / 时间窗防重放？ | P6 |

> **批准后**：按 P0 → P13 顺序实施；每阶段提交 + 更新验收清单；遇到与本文档
> 冲突的新事实，先改文档再改代码。

---

## 12. 附录 A：公开类型契约（已落地）

```rust
// src/lib.rs
#[async_trait]
pub trait ChainSdk: Send + Sync {
    async fn health_check(&self) -> Result<HealthStatus, ChainSdkError>;
    async fn get_anchor(&self, anchor_id: &str) -> Result<AnchorRecord, ChainSdkError>;
    async fn list_anchors(&self, anchor_type: Option<AnchorType>, submitter_id: Option<u32>)
        -> Result<Vec<AnchorRecord>, ChainSdkError>;
    async fn get_participant(&self, participant_id: u32) -> Result<ParticipantRecord, ChainSdkError>;
    async fn get_participant_by_did(&self, did: &str) -> Result<Option<ParticipantRecord>, ChainSdkError>;
    async fn list_participants(&self) -> Result<Vec<ParticipantRecord>, ChainSdkError>;
    async fn resolve_did(&self, did: &str) -> Result<Option<DidRecord>, ChainSdkError>;
    fn resolve_did_key(&self, did: &str) -> Result<Ed25519PublicKey, ChainSdkError>;
    async fn resolve_did_web(&self, did: &str) -> Result<DidDocument, ChainSdkError>;
    async fn create_anchor(&self, anchor_type: AnchorType, payload_hash: &str, valid_until: u64)
        -> Result<String, ChainSdkError>;
    async fn revoke_anchor(&self, anchor_id: &str) -> Result<(), ChainSdkError>;
    async fn renew_anchor(&self, anchor_id: &str, new_valid_until: u64) -> Result<(), ChainSdkError>;
    async fn update_authorized_domains(&self, participant_id: u32, domains: Vec<String>)
        -> Result<(), ChainSdkError>;
    async fn update_controller_did(&self, participant_id: u32, new_controller_did: &str)
        -> Result<(), ChainSdkError>;
    async fn sign_participant_assertion(&self, payload: &[u8], purpose: AssertionPurpose)
        -> Result<Ed25519Signature, ChainSdkError>;
    fn subscribe_event(&self, event_name: &str, callback: Box<dyn Fn(EventData) + Send + Sync>)
        -> Result<Unsubscribe, ChainSdkError>;
}

pub type Unsubscribe = Box<dyn FnOnce() + Send + Sync>;
```

配置（扁平字段，TOML 嵌套由 `FileConfig` 映射）：

```rust
pub struct ChainSdkConfig {
    pub mode: ChainSdkMode,               // mock / centralized / chain
    pub role: SdkRole,                    // verifier / issuer（app 别名）
    pub participant_id: u32,
    pub key_vault: KeyVaultConfig,
    pub participant: ParticipantConfig,   // controller_did
    pub did: DidConfig,                   // vc_storage_path
    pub mock: MockConfig,                 // state_path: Option<PathBuf>
    pub capabilities: Capabilities,
    pub centralized_endpoint: String,
    pub chain_endpoint: String,
    pub require_finalized_node: bool,
    pub cache_ttl_secs: u64,
    pub valid_until_refresh_window_secs: u64,
    #[serde(skip)] pub resolved_password: Option<secrecy::SecretString>,
}
```

---

## 13. 附录 B：配置解析算法

```text
resolve_config(overrides):
  1. cfg = ChainSdkConfig::default()            # home = CHAIN_SDK_HOME / data_dir/chain-sdk
  2. if CHAIN_SDK_CONFIG_FILE or {home}/chain-sdk.toml exists:
         parse FileConfig（全 Option），逐段覆盖
  3. apply_env(cfg)                             # CHAIN_SDK_* 白名单
  4. apply_overrides(cfg, overrides)
  5. cfg.capabilities = cfg.capabilities.effective_for(cfg.role)
  6. if cfg.role.needs_keyvault():
         cfg.resolved_password = resolve_password(cfg.key_vault)
     else:
         cfg.resolved_password = None
  7. validate: participant_id > 0? controller_did 非空（Issuer）?
  8. return cfg
```

**密码链（仅 Issuer，`scheme = "file-keystore"`）**：

```text
overrides.key_vault_password
  → env(password_env)（默认 CHAIN_KEYSTORE_PASSWORD）
  → password_file（存在则读）
  → allow_auto_generate_password（生成 32B，写 0600）
  → ConfigError
```

`scheme = "memory-keystore"` 不解析密码。

**TOML 段映射**：

| TOML | 结构体字段 |
|---|---|
| `[chain_sdk]` mode/role/participant_id/cache_ttl_secs/valid_until_refresh_window_secs/require_finalized_node | 同名 |
| `[chain_sdk.key_vault]` + `[chain_sdk.key_vault.keys]` | `key_vault` / `key_vault.keys` |
| `[chain_sdk.participant]` controller_did | `participant.controller_did` |
| `[chain_sdk.did]` vc_storage_path | `did.vc_storage_path` |
| `[chain_sdk.mock]` state_path | `mock.state_path` |
| `[chain_sdk.capabilities]` | `capabilities` |
| `[chain_sdk.centralized]` endpoint | `centralized_endpoint` |
| `[chain_sdk.chain]` endpoint / require_finalized_node | `chain_endpoint` / `require_finalized_node` |

---

## 14. 附录 C：Centralized HTTP / WebSocket 契约（提议）

> **状态**：待 Q-01 确认；若中心化服务已有契约，以对方为准并更新本附录。

### C.1 端点

| 操作 | Method + Path | 请求体 | 响应 |
|---|---|---|---|
| 健康 | `GET /health` | — | `{healthy, block_number}` |
| 锚定读 | `GET /anchors/{anchor_id}` | — | `AnchorRecord` |
| 锚定列 | `GET /anchors?anchor_type={kind}&submitter_id={id}` | — | `[AnchorRecord]` |
| 锚定写 | `POST /anchors` | `{anchor_type, payload_hash, valid_until}` | `{anchor_id}` |
| 撤销 | `POST /anchors/{id}/revoke` | `{}` | `{}` |
| 续期 | `POST /anchors/{id}/renew` | `{valid_until}` | `{}` |
| 参与方列 | `GET /participants` | — | `[ParticipantRecord]` |
| 参与方读 | `GET /participants/{id}` | — | `ParticipantRecord` |
| 按 DID | `GET /participants/by-did/{did}` | — | `ParticipantRecord` / 404 |
| 域名写 | `POST /participants/{id}/domains` | `{domains: [string]}` | `{}` |
| controller 写 | `POST /participants/{id}/controller-did` | `{controller_did}` | `{}` |
| DID 读 | `GET /dids/{did}` | — | `DidRecord` / 404 |
| 事件 | `GET /events`（Upgrade: websocket） | — | `EventData` JSON 帧 |

### C.2 写请求签名

```text
canonical = op || "\n" || canonical_json(body)
signature = sr25519_sign(chain-account/<id>, canonical)   # keyvault-core 内部加 "substrate" 上下文
headers:
  x-chain-signer:    "chain-account/<id>"
  x-chain-signature: hex(signature)
  x-chain-op:        op
```

`op` 取值：`create_anchor` / `revoke_anchor` / `renew_anchor` /
`update_authorized_domains` / `update_controller_did`。

### C.3 canonical JSON

- 对象键按字典序递归升序；
- 无多余空白（`,` / `:` 紧凑）；
- 只允许 `string` / `integer` / `bool` / `null` / 数组 / 对象；**拒绝浮点**；
- 字符串按 JSON 标准转义，不转义非 ASCII（UTF-8 原样）；
- 顶层必须是对象。

POC 实现：`serde_json::to_vec` + 递归排序 `Value::Object`（`BTreeMap` 作为中间结构）。
若 Q-03 要求 RFC 8785，则替换为 JCS 实现并加共享向量。

### C.4 错误映射

| HTTP | `ChainSdkError` |
|---|---|
| 404 | `AnchorNotFound` / `ParticipantNotFound` |
| 401/403 | `Unauthorized` |
| 409 | `AnchorRevoked` / `ParticipantInactive` |
| 5xx / 连接失败 | `ChainUnavailable` / `NetworkError` |
| 400 | `InvalidArgument`（取响应 `reason`） |

---

## 15. 附录 D：SD-JWT（`sd-v1`，待 D-13～D-16 批准）

### D.1 VC 结构

```json
{
  "@context": ["https://www.w3.org/ns/credentials/v2"],
  "type": ["VerifiableCredential", "IDCardVC"],
  "id": "urn:...",
  "issuer": "did:web:issuer.example.com",
  "issuanceDate": "2026-01-01T00:00:00Z",
  "expirationDate": "2036-01-01T00:00:00Z",
  "credentialSubject": {
    "id": "did:key:zHolder",
    "idCardNumber": "110101199001011234",
    "_sd": ["zWHyPrbD-V_LExFdXsIIt2FG6ENDSaMwFUKmn2s5qKk"],
    "_sd_alg": "sha-256"
  },
  "credentialStatus": { "type": "AnchorStatus", "anchorId": "anchor:1" },
  "proof": { "type": "Ed25519Signature2020", "created": 0,
             "verificationMethod": "did:web:issuer.example.com#key-1", "jws": "..." }
}
```

### D.2 哈希与 Disclosure（`sd-v1`）

```text
salt_raw   = 16 bytes CSPRNG
salt       = base64url_nopad(salt_raw)
H_i        = SHA-256( salt || base64url_nopad(utf8(name)) || base64url_nopad(utf8(value)) )
_sd[i]     = base64url_nopad(H_i)
Disclosure = [salt, name, value]            # JSON 数组
```

**Golden vector（review 后锁定）**：

```text
salt_raw_hex = 000102030405060708090a0b0c0d0e0f
salt         = AAECAwQFBgcICQoLDA0ODw
name         = name                       → bmFtZQ
value        = 张三                        → 5byg5LiJ
digest_input = AAECAwQFBgcICQoLDA0ODwbmFtZQ5byg5LiJ
_sd[0]       = zWHyPrbD-V_LExFdXsIIt2FG6ENDSaMwFUKmn2s5qKk
```

（若 D-14 改为 RFC 9901，此向量替换为
`disclosure = WyJBQUVDQXdRRkJnY0lDUW9MREEwT0R3IiwibmFtZSIsIuW8oOS4iSJd`，
`digest = 3Y-DVS8_dEVMQcQpTBHxGdYEJq_Ev-H2y06VQ8YjMkw`。）

### D.3 VP 结构（`SD-VP-v1`）

```json
{
  "@context": ["https://www.w3.org/ns/credentials/v2"],
  "type": ["VerifiablePresentation"],
  "holder": "did:key:zHolder",
  "verifiableCredential": [VC],
  "disclosures": [["AAEC...", "name", "张三"]],
  "challenge": "...",
  "domain": "...",
  "proof": { "type": "Ed25519Signature2020", "verificationMethod": "did:key:zHolder#key-1",
             "jws": "..." }
}
```

签名输入：

```text
SD-VP-v1
{holder}
{challenge}
{domain}
{sha256(b64url(compact VC JSON))}
{sha256(b64url(compact disclosure JSON))}   # 按 disclosure 顺序或排序，二者择一并在测试锁定
```

> 采用**排序后的 disclosure 哈希**（升序、每行一个），避免顺序歧义；与 did-sdk
> `VP-v1` 不兼容，因此必须用 `SD-VP-v1` 独立输入（D-16）。

### D.4 验证算法

```text
1. 校验 challenge / domain
2. 重算 SD-VP-v1 输入 → 用 holder DID 的公钥验签（did:key 离线 / did:web HTTPS）
3. 用 issuer DID 校验 VC proof（IssuedVc-v1 输入）
4. 对每条 disclosure 重算 H_i：
     - 不在 _sd → DisclosureHashMismatch
     - 同一 name 重复 → DisclosureDuplicate
     - 缺失 disclosures → DisclosureMissing
     - _sd_alg != "sha-256" → SdAlgUnsupported
5. get_anchor(credentialStatus.anchorId)：
     - revoked → AnchorRevoked
     - valid_until != 0 && now > valid_until → AnchorExpired（用 AnchorRevoked/自定义？见 D-15 备注）
6. 重算 payload_hash（blake2b-256，VC 去 proof）并与锚定值比对
7. 返回 SdVerificationReport { verified, disclosed, undisclosed_count, ... }
```

> 注：`AnchorExpired` 不在规格一的 `ChainSdkError` 中；建议用
> `ChainUnavailable`? **不**。P12 落地时新增 `AnchorExpired(String)` 变体（与
> `AnchorRevoked` 对称），并在 D-15 中一并批准。

### D.5 隐私边界（与附件 §5 一致）

| 能防 | 不能防 |
|---|---|
| 验证方看到未披露属性 | 同一 VC 跨出示关联（`_sd` 哈希固定） |
| 篡改被披露属性 | 多验证方合谋拼接 |
| 伪造未披露属性存在 | 验证方长期存储已披露属性 |
| 跨 VC 关联（独立盐） | — |
| 字典攻击（随机盐） | — |

---

## 16. 附录 E：常用命令与环境

```bash
# 本机（沙箱）通用前缀
export RUSTUP_HOME=/Users/metercai/Desktop/chains/did-sdk/.rustup
export CARGO_HOME=/Users/metercai/Desktop/chains/chain-sdk/.cargo-home
RUSTUP=/Users/metercai/.cargo/bin/rustup

# 检查 / 测试
$RUSTUP run 1.88.0 cargo check --locked --all-targets --all-features
$RUSTUP run 1.88.0 cargo fmt --all -- --check
$RUSTUP run 1.88.0 cargo clippy --locked --all-targets --all-features -- -D warnings
$RUSTUP run 1.88.0 cargo test --locked --all-features

# 角色边界
bash scripts/verify-roles.sh
bash scripts/run-consistency.sh

# musl 静态（CI / 有 zig 的机器）
bash scripts/build-rust-musl.sh

# 查看依赖边界
$RUSTUP run 1.88.0 cargo tree -p did-sdk | grep subxt        # 期望无输出
$RUSTUP run 1.88.0 cargo tree | grep -E 'openssl|native-tls' # 期望无输出
```

CI 无需上述前缀；`.cargo/config.toml` 不提交。

---

## 17. 变更控制

1. 本文档是实施的唯一依据；实现中发现的规格冲突，**先改本文档**并记录决策，
   再改代码；
2. 每个阶段一个提交，提交信息 `P<N>: <主题>`，附带门禁命令结果；
3. 验收证据写入 `docs/acceptance-checklist.md`（命令 + 结果摘要 + 提交号）；
4. 破坏性 API 变更（`ChainSdk` trait、`ChainSdkError`、配置字段）必须记入
   §4 决策并在 README 同步；
5. GitHub Actions 失败即视为阶段未完成。
