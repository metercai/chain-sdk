# Chain 元数据说明

本目录**不提交** `identity_chain.scale`。

原因（见 `docs/implementation-plan.md` 决策 D-06）：

1. 仓库内没有 identity-chain 的 pallet 名称、存储项名称与 SCALE 类型定义；
2. 提交错误或过期的元数据会让 `ChainModeSdk` 编译期通过、运行时全部失败；
3. `subxt 0.50` 支持连接节点后**在线获取 metadata**，Chain 后端按运行时元数据
   做动态 storage 查询。

若后续从链维护方拿到真实元数据，推荐流程：

```bash
# 从节点导出（命令以链维护方提供为准）
subxt metadata --url ws://127.0.0.1:9944 -o metadata/identity_chain.scale
```

把文件提交到本目录，并在 `src/chain.rs` 中启用静态元数据路径（`subxt::OnlineClient`
使用 `from_rpc` + 静态 metadata）。在此之前，Chain 后端：

- 读路径：动态元数据查询；
- 写路径：若 P10 实测 extrinsic 签名桥接不可行，返回
  `ChainSdkError::Unsupported`（规格允许 POC 占位）；
- 角色拦截（S9 / S17 / S19）**必须**在任何链调用之前生效。
