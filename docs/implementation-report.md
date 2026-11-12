# chain-sdk 实施记录（P0–P12）

| 项目 | 内容 |
|---|---|
| 仓库 | [`metercai/chain-sdk`](https://github.com/metercai/chain-sdk)（`main` 已推送） |
| 版本 | `0.1.0` |
| 工具链 | Rust 1.88.0，edition 2024，`resolver = "3"` |
| 本地门禁 | `cargo fmt --check`、`cargo clippy -D warnings`、`cargo test` **93 passed** |
| 依据 | `docs/implementation-plan.md`（D-01～D-20 已批准） |
| 状态 | **P0–P13 完成**：本地 93 测试通过；4 个 GitHub Actions workflow 全绿 |

---

## 1. 交付物

```text
src/
├── lib.rs              ChainSdk trait（17 方法）+ SdkCore + 重新导出
├── config.rs           配置解析（TOML 嵌套段 / env / overrides / 密码链）
├── types.rs            数据模型（+ AnchorType::as_str）
├── error.rs            ChainSdkError（24 变体）+ map_vault_err
├── events.rs           SubscriptionTable（std RwLock、通配、幂等取消）
├── keys.rs             两类密钥、namespace 唯一入口（S14）
├── role_gate.rs        ensure_issuer / ensure_self / ensure_signer（S9/S19）
├── did_adapter.rs      DidPort + DidSdk<Server> 适配（S17）
├── mock.rs             Mock 后端（读/写/持久化/故障注入/事件）
├── centralized.rs      Centralized 后端（HTTP + TTL + 签名 + WebSocket）
├── chain.rs            Chain 后端（subxt 连接/健康/事件 + 角色拦截）
└── sd.rs               SD-JWT（签发/验证/签名输入/payload_hash）

tests/                  16 个验收测试文件 + fixtures
examples/mini_cli.rs    init-keys / health / get-anchor / create-anchor
docs/                   implementation-plan / acceptance-checklist / implementation-report / specs
configs/ scripts/ .github/workflows/ metadata/
```

## 2. 阶段与提交

| 阶段 | 主题 | 提交 |
|---|---|---|
| 初始化 | 骨架 + 契约 + CI + 实施文档 | `112af3c` |
| P0 | 配置解析、namespace、角色门禁 | `abd3b5e` |
| P1–P2 | Mock 读写、持久化、故障注入 | `eac7848` |
| P3–P4 | did_adapter、按角色装配 | `4f35b80` |
| P5–P6 | Centralized 读写、缓存、签名 | `e10585f` |
| P7–P8 | 参与方绑定、委托锚定、无治理 | （提交见 `git log`） |
| P9 | 三后端事件订阅 | `a8506af` |
| P10–P11 | Chain 后端、mini_cli、一致性 | `e9c62f8` |
| P12 | SD-JWT 原语 | `6bdc892` |

## 3. 验收证据（本地）

| 检查 | 命令 | 结果 |
|---|---|---|
| 编译 | `cargo check --locked --all-targets --all-features` | 通过 |
| 格式 | `cargo fmt --all -- --check` | 通过 |
| Lint | `cargo clippy --locked --all-targets --all-features -- -D warnings` | 零告警 |
| 测试 | `cargo test --locked --all-features` | **93 passed, 0 failed** |
| 角色边界 | `bash scripts/verify-roles.sh` | 通过（S18/S17 grep） |
| 一致性 | `bash scripts/run-consistency.sh` | 通过 |
| 依赖边界 | `cargo tree -p did-sdk \| grep subxt` | 无输出（R1） |
| 禁止 TLS | `cargo tree \| grep -E 'openssl\|native-tls'` | 无输出 |
| mini_cli | `CHAIN_SDK_HOME=<tmp> cargo run --example mini_cli init-keys` | 打印真实 `KeyId` |

> musl 静态（`x86_64`/`aarch64`）与 Windows/macOS 交叉构建由 GitHub Actions
> `rust-musl.yml` 执行；本地无 zig/cargo-zigbuild。

### 3.1 GitHub Actions（commit `main` 首推）

| workflow | 结果 | 链接 |
|---|---|---|
| `ci`（fmt / clippy / test / 契约） | ✅ success | https://github.com/metercai/chain-sdk/actions/runs/43274367793 |
| `rust-musl`（4 target，musl 静态断言） | ✅ success | https://github.com/metercai/chain-sdk/actions/runs/43274367936 |
| `consistency` | ✅ success | https://github.com/metercai/chain-sdk/actions/runs/43274367885 |
| `roles`（角色矩阵 + S18/S17 grep） | ✅ success | https://github.com/metercai/chain-sdk/actions/runs/43274367598 |

## 4. 与规格/计划的实施偏差（均已在计划中记录）

1. **MSRV 1.88**（D-01）：did-sdk 与 subxt 的硬要求。
2. **did-sdk 适配**（D-02）：`DidSdk<M>` 是具体泛型，不能 `dyn`；内部 `DidPort` 收敛。
3. **`participant_did()` 访问器**（D-13 实施记录）：SD 签发需要 issuer DID；
   `ChainSdk` 方法数 16 → 17，CI 契约检查同步。
4. **两角色都要求 `controller_did`**（D-05 补充）：did-sdk Server 构造要求。
5. **`payload_hash` 排除 `credentialStatus`**（D-15 实施补充）：避免“先有锚定 id
   才能算哈希”的循环；`credentialStatus` 由 issuer 签名覆盖。
6. **Chain 读写**（D-06）：连接/健康/事件/角色拦截已实现；storage 查询与
   extrinsic 提交返回 `Unsupported`（缺元数据 + subxt `Signer` 不可失败）。
7. **`sd-jwt` feature**：保留为未来门控；当前实现始终编译，避免两套代码路径。

## 5. 外部依赖与未完成项

| 项 | 依赖 | 影响 |
|---|---|---|
| ~~GitHub 推送 / 4 个 workflow 全绿~~ | ✅ 已完成（osxkeychain 凭据） | — |
| Chain storage / extrinsic | identity-chain pallet 元数据与节点 | S2/S3 的 chain 读、真实链验收 |
| SD-JWT 端到端（I6 / D1–D5） | did-sdk v0.2（`credentialStatus`、`disclosures`、VP 签名输入、未知字段保留） | 持有者存储/出示 |
| Centralized 服务契约 | Q-01（服务端实现/规格） | 端点与错误码最终对齐 |
| canonical JSON | Q-03（是否 RFC 8785） | 跨语言签名一致性 |

## 6. 下一步

1. 拿到 identity-chain pallet 元数据后补 Chain storage 查询与真实链集成测试；
2. 与 did-sdk 排期 v0.2，完成 SD-JWT 端到端闭环（Disclosures 存储/出示）；
3. 与中心化模拟链服务对齐附录 C 契约（Q-01）；
4. 评估 RFC 8785 JCS canonical JSON（Q-03）与 Centralized 重放保护（Q-07）。
