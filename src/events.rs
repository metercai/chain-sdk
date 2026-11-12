//! 事件订阅的数据结构与取消句柄。

use serde::{Deserialize, Serialize};

/// 链上事件。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventData {
    /// 事件名，例如 `AnchorCreated` / `ParticipantUpdated`。
    pub event_name: String,
    /// 关联的锚定 id。
    pub anchor_id: Option<String>,
    /// 关联的参与方 id。
    pub participant_id: Option<u32>,
    /// 区块高度（Mock 为内部时间线序号）。
    pub block_number: Option<u64>,
    /// 事件载荷（后端原始字段）。
    pub payload: serde_json::Value,
}

/// 取消订阅句柄：调用一次即从回调表移除；重复调用无副作用。
pub type Unsubscribe = Box<dyn FnOnce() + Send + Sync>;
