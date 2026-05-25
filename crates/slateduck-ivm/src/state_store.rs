//! Per-shard SlateDB state store path management.
//!
//! Each shard owns an isolated SlateDB sub-directory under the matview's
//! `state_uri` prefix:
//!
//! ```text
//! {state_prefix}/matviews/{matview_id}/shards/{shard_id}/
//! ```
//!
//! This module provides path helpers and the shard state store abstraction.
//! The actual SlateDB instance is opened lazily on first use.

/// Compute the object-store path prefix for a shard's state store.
///
/// # Arguments
/// * `state_prefix` — the matview's top-level state URI (from `MatviewRow.state_uri`).
/// * `matview_id`   — the matview identifier.
/// * `shard_id`     — the shard identifier.
pub fn shard_state_path(state_prefix: &str, matview_id: u64, shard_id: u32) -> String {
    let prefix = state_prefix.trim_end_matches('/');
    format!("{prefix}/matviews/{matview_id}/shards/{shard_id}")
}

/// Compute the checkpoint file path within a shard's state store.
pub fn shard_checkpoint_path(state_prefix: &str, matview_id: u64, shard_id: u32) -> String {
    format!(
        "{}/checkpoint",
        shard_state_path(state_prefix, matview_id, shard_id)
    )
}

/// A handle to a shard's isolated state store.
///
/// In v0.12 the state store is used to persist the DBSP circuit state
/// (aggregate accumulators) between worker restarts.  The store is opened on
/// [`ShardStateStore::open`] and closed on drop.
pub struct ShardStateStore {
    pub path: String,
}

impl ShardStateStore {
    /// Return the path for this shard's state store without opening it.
    pub fn new(state_prefix: &str, matview_id: u64, shard_id: u32) -> Self {
        Self {
            path: shard_state_path(state_prefix, matview_id, shard_id),
        }
    }

    /// Return the checkpoint key path for this shard.
    pub fn checkpoint_path(&self) -> String {
        format!("{}/checkpoint", self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shard_state_path_is_deterministic() {
        assert_eq!(
            shard_state_path("s3://bucket/state", 42, 3),
            "s3://bucket/state/matviews/42/shards/3"
        );
    }

    #[test]
    fn shard_checkpoint_path() {
        assert_eq!(
            super::shard_checkpoint_path("s3://bucket/state", 1, 0),
            "s3://bucket/state/matviews/1/shards/0/checkpoint"
        );
    }

    #[test]
    fn trailing_slash_is_normalised() {
        assert_eq!(
            shard_state_path("s3://bucket/state/", 1, 0),
            "s3://bucket/state/matviews/1/shards/0"
        );
    }
}
