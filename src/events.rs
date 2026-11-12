//! 事件订阅的数据结构与回调表。
//!
//! 回调表用 `std::sync::RwLock`（`subscribe_event` 是同步方法）；`emit` 时先把
//! 匹配的回调克隆到局部 `Vec`，释放锁后再调用，**不跨 `.await` 持锁**。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

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

/// 取消订阅句柄：调用一次即从回调表移除。
pub type Unsubscribe = Box<dyn FnOnce() + Send + Sync>;

struct Subscription {
    id: u64,
    event_name: String,
    callback: Arc<dyn Fn(EventData) + Send + Sync>,
}

/// 进程内回调表（三个后端共享）。
#[derive(Default)]
pub(crate) struct SubscriptionTable {
    subscriptions: RwLock<Vec<Subscription>>,
    next_id: AtomicU64,
}

impl SubscriptionTable {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// 注册订阅；`event_name = "*"` 表示全部事件。
    ///
    /// 返回的句柄被调用时移除该订阅；重复注册互不影响。
    pub(crate) fn register(
        self: &Arc<Self>,
        event_name: &str,
        callback: Box<dyn Fn(EventData) + Send + Sync>,
    ) -> Unsubscribe {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        {
            let mut subscriptions = self
                .subscriptions
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            subscriptions.push(Subscription {
                id,
                event_name: event_name.to_string(),
                callback: Arc::from(callback),
            });
        }
        let table = Arc::clone(self);
        Box::new(move || table.remove(id))
    }

    fn remove(&self, id: u64) {
        let mut subscriptions = self
            .subscriptions
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        subscriptions.retain(|subscription| subscription.id != id);
    }

    /// 分发事件：匹配 `event_name` 或通配 `"*"` 的回调。
    pub(crate) fn emit(&self, event: &EventData) {
        let callbacks: Vec<Arc<dyn Fn(EventData) + Send + Sync>> = {
            let subscriptions = self
                .subscriptions
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            subscriptions
                .iter()
                .filter(|subscription| {
                    subscription.event_name == event.event_name || subscription.event_name == "*"
                })
                .map(|subscription| Arc::clone(&subscription.callback))
                .collect()
        };
        for callback in callbacks {
            callback(event.clone());
        }
    }

    /// 当前订阅数（测试与诊断用）。
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.subscriptions
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::sync::{Arc, Mutex};

    use super::*;

    fn event(name: &str) -> EventData {
        EventData {
            event_name: name.to_string(),
            anchor_id: Some("a1".into()),
            participant_id: Some(4),
            block_number: Some(1),
            payload: serde_json::json!({}),
        }
    }

    #[test]
    fn table_delivers_and_unsubscribe_stops() {
        let table = Arc::new(SubscriptionTable::new());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let callback = {
            let seen = Arc::clone(&seen);
            Box::new(move |event: EventData| {
                seen.lock().unwrap().push(event.event_name);
            })
        };
        let unsubscribe = table.register("AnchorCreated", callback);
        assert_eq!(table.len(), 1);

        table.emit(&event("AnchorCreated"));
        table.emit(&event("Other"));
        assert_eq!(*seen.lock().unwrap(), vec!["AnchorCreated"]);

        unsubscribe();
        assert_eq!(table.len(), 0);
        table.emit(&event("AnchorCreated"));
        assert_eq!(seen.lock().unwrap().len(), 1);
    }

    #[test]
    fn wildcard_matches_all_events() {
        let table = Arc::new(SubscriptionTable::new());
        let seen = Arc::new(Mutex::new(0usize));
        let callback = {
            let seen = Arc::clone(&seen);
            Box::new(move |_event: EventData| {
                *seen.lock().unwrap() += 1;
            })
        };
        let _unsubscribe = table.register("*", callback);
        table.emit(&event("A"));
        table.emit(&event("B"));
        assert_eq!(*seen.lock().unwrap(), 2);
    }
}
