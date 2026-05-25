//! LISTEN/NOTIFY/UNLISTEN: pub-sub channel support for event-driven scheduling.
//!
//! pg-trickle uses `LISTEN pgt_source_changed_<table_id>` to wake immediately
//! when a snapshot advances. Without this, pg-trickle falls back to polling.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio::sync::RwLock;

/// A notification message sent to subscribers.
#[derive(Debug, Clone)]
pub struct Notification {
    /// The channel name (e.g., `pgt_source_changed_42`).
    pub channel: String,
    /// Optional payload.
    pub payload: String,
}

/// Manages LISTEN/NOTIFY subscriptions across all connections.
pub struct NotifyManager {
    /// Per-channel broadcast senders.
    channels: RwLock<HashMap<String, broadcast::Sender<Notification>>>,
}

impl NotifyManager {
    pub fn new() -> Self {
        Self {
            channels: RwLock::new(HashMap::new()),
        }
    }

    /// Subscribe to a channel. Returns a receiver for notifications.
    pub async fn listen(&self, channel: &str) -> broadcast::Receiver<Notification> {
        let mut channels = self.channels.write().await;
        let sender = channels
            .entry(channel.to_string())
            .or_insert_with(|| broadcast::channel(64).0);
        sender.subscribe()
    }

    /// Send a notification to all listeners on a channel.
    pub async fn notify(&self, channel: &str, payload: &str) {
        let channels = self.channels.read().await;
        if let Some(sender) = channels.get(channel) {
            let _ = sender.send(Notification {
                channel: channel.to_string(),
                payload: payload.to_string(),
            });
        }
    }

    /// Emit notifications for a snapshot advance affecting a set of table IDs.
    pub async fn notify_snapshot_advance(&self, table_ids: &[u64]) {
        for table_id in table_ids {
            let channel = format!("pgt_source_changed_{table_id}");
            self.notify(&channel, "").await;
        }
    }
}

impl Default for NotifyManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-connection subscription state.
pub struct ConnectionSubscriptions {
    /// Channels this connection is listening on.
    pub channels: HashSet<String>,
    /// Receivers for each subscribed channel.
    pub receivers: HashMap<String, broadcast::Receiver<Notification>>,
}

impl ConnectionSubscriptions {
    pub fn new() -> Self {
        Self {
            channels: HashSet::new(),
            receivers: HashMap::new(),
        }
    }

    /// Subscribe to a channel. Returns true if newly subscribed.
    pub async fn listen(&mut self, channel: &str, manager: &Arc<NotifyManager>) -> bool {
        if self.channels.contains(channel) {
            return false;
        }
        let rx = manager.listen(channel).await;
        self.channels.insert(channel.to_string());
        self.receivers.insert(channel.to_string(), rx);
        true
    }

    /// Unsubscribe from a channel. Returns true if was subscribed.
    pub fn unlisten(&mut self, channel: &str) -> bool {
        self.receivers.remove(channel);
        self.channels.remove(channel)
    }

    /// Unsubscribe from all channels.
    pub fn unlisten_all(&mut self) {
        self.channels.clear();
        self.receivers.clear();
    }

    /// Drain any pending notifications from all subscribed channels.
    pub fn drain_notifications(&mut self) -> Vec<Notification> {
        let mut notifications = Vec::new();
        for (_channel, rx) in self.receivers.iter_mut() {
            while let Ok(notif) = rx.try_recv() {
                notifications.push(notif);
            }
        }
        notifications
    }
}

impl Default for ConnectionSubscriptions {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_listen_notify_roundtrip() {
        let manager = Arc::new(NotifyManager::new());

        let mut subs = ConnectionSubscriptions::new();
        subs.listen("test_channel", &manager).await;

        manager.notify("test_channel", "hello").await;

        let notifs = subs.drain_notifications();
        assert_eq!(notifs.len(), 1);
        assert_eq!(notifs[0].channel, "test_channel");
        assert_eq!(notifs[0].payload, "hello");
    }

    #[tokio::test]
    async fn test_unlisten() {
        let manager = Arc::new(NotifyManager::new());

        let mut subs = ConnectionSubscriptions::new();
        subs.listen("ch1", &manager).await;
        assert!(subs.unlisten("ch1"));
        assert!(!subs.unlisten("ch1")); // Already removed

        manager.notify("ch1", "ignored").await;
        let notifs = subs.drain_notifications();
        assert!(notifs.is_empty());
    }

    #[tokio::test]
    async fn test_notify_snapshot_advance() {
        let manager = Arc::new(NotifyManager::new());

        let mut subs = ConnectionSubscriptions::new();
        subs.listen("pgt_source_changed_42", &manager).await;
        subs.listen("pgt_source_changed_99", &manager).await;

        manager.notify_snapshot_advance(&[42, 99, 100]).await;

        let notifs = subs.drain_notifications();
        assert_eq!(notifs.len(), 2);
    }

    #[tokio::test]
    async fn test_multiple_subscribers() {
        let manager = Arc::new(NotifyManager::new());

        let mut subs1 = ConnectionSubscriptions::new();
        let mut subs2 = ConnectionSubscriptions::new();
        subs1.listen("shared", &manager).await;
        subs2.listen("shared", &manager).await;

        manager.notify("shared", "broadcast").await;

        let n1 = subs1.drain_notifications();
        let n2 = subs2.drain_notifications();
        assert_eq!(n1.len(), 1);
        assert_eq!(n2.len(), 1);
    }
}
