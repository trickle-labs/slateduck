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

// ─── v0.18: Mixed Frontier ─────────────────────────────────────────────────

use std::collections::BTreeMap;

/// Identifier for a data source in a mixed frontier.
pub type SourceId = String;

/// A frontier value for a single source. Heterogeneous across source types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceFrontier {
    /// DuckLake snapshot-based frontier (fully understood by SlateDuck).
    DuckLakeSnapshot(i64),
    /// Internal sequence number (e.g., IVM checkpoint seq).
    SequenceNumber(u64),
    /// Opaque bytes (e.g., PostgreSQL WAL LSN). SlateDuck persists but never interprets.
    Opaque(Vec<u8>),
}

/// A mixed frontier: vector clock over heterogeneous source types.
///
/// For stream tables that read from both DuckLake and PostgreSQL sources,
/// each source has its own frontier component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MixedFrontier {
    pub sources: BTreeMap<SourceId, SourceFrontier>,
}

impl MixedFrontier {
    pub fn new() -> Self {
        Self {
            sources: BTreeMap::new(),
        }
    }

    /// Set the frontier for a source.
    pub fn set(&mut self, source_id: SourceId, frontier: SourceFrontier) {
        self.sources.insert(source_id, frontier);
    }

    /// Get the frontier for a source.
    pub fn get(&self, source_id: &str) -> Option<&SourceFrontier> {
        self.sources.get(source_id)
    }

    /// Get the DuckLake snapshot component, if any source has one.
    pub fn ducklake_snapshot(&self) -> Option<i64> {
        self.sources.values().find_map(|f| match f {
            SourceFrontier::DuckLakeSnapshot(snap) => Some(*snap),
            _ => None,
        })
    }

    /// Serialize to JSON for observability. Opaque values are base64-encoded.
    pub fn to_json(&self) -> String {
        use base64::Engine;
        let entries: Vec<String> = self
            .sources
            .iter()
            .map(|(source_id, frontier)| {
                let value = match frontier {
                    SourceFrontier::DuckLakeSnapshot(snap) => {
                        format!("{{\"type\":\"ducklake_snapshot\",\"value\":{snap}}}")
                    }
                    SourceFrontier::SequenceNumber(seq) => {
                        format!("{{\"type\":\"sequence_number\",\"value\":{seq}}}")
                    }
                    SourceFrontier::Opaque(bytes) => {
                        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
                        format!("{{\"type\":\"opaque\",\"value\":\"{encoded}\"}}")
                    }
                };
                format!("\"{source_id}\":{value}")
            })
            .collect();
        format!("{{{}}}", entries.join(","))
    }

    /// Deserialize from JSON.
    pub fn from_json(json: &str) -> Result<Self, String> {
        use base64::Engine;
        let mut frontier = Self::new();

        // Simple JSON parser for the known format.
        let trimmed = json.trim().trim_start_matches('{').trim_end_matches('}');
        if trimmed.is_empty() {
            return Ok(frontier);
        }

        // Split by top-level entries (key:value pairs).
        let mut depth = 0i32;
        let mut start = 0;
        let mut entries = Vec::new();
        for (i, ch) in trimmed.char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => depth -= 1,
                ',' if depth == 0 => {
                    entries.push(&trimmed[start..i]);
                    start = i + 1;
                }
                _ => {}
            }
        }
        entries.push(&trimmed[start..]);

        for entry in entries {
            let entry = entry.trim();
            // Find the source_id (first quoted string).
            let id_start = entry.find('"').ok_or("missing source id")? + 1;
            let id_end = entry[id_start..].find('"').ok_or("missing source id end")? + id_start;
            let source_id = entry[id_start..id_end].to_string();

            // Find the type.
            if let Some(type_pos) = entry.find("\"type\":\"") {
                let type_start = type_pos + "\"type\":\"".len();
                let type_end =
                    entry[type_start..].find('"').ok_or("missing type end")? + type_start;
                let type_str = &entry[type_start..type_end];

                match type_str {
                    "ducklake_snapshot" => {
                        if let Some(val) = extract_json_number(entry, "value") {
                            frontier.set(source_id, SourceFrontier::DuckLakeSnapshot(val as i64));
                        }
                    }
                    "sequence_number" => {
                        if let Some(val) = extract_json_number(entry, "value") {
                            frontier.set(source_id, SourceFrontier::SequenceNumber(val));
                        }
                    }
                    "opaque" => {
                        if let Some(val_str) = extract_json_string(entry, "value") {
                            let bytes = base64::engine::general_purpose::STANDARD
                                .decode(val_str)
                                .map_err(|e| e.to_string())?;
                            frontier.set(source_id, SourceFrontier::Opaque(bytes));
                        }
                    }
                    _ => {}
                }
            }
        }

        Ok(frontier)
    }
}

impl Default for MixedFrontier {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract a numeric value for a given key from a JSON fragment.
fn extract_json_number(json: &str, key: &str) -> Option<u64> {
    let pattern = format!("\"{}\":", key);
    let pos = json.find(&pattern)? + pattern.len();
    let rest = json[pos..].trim();
    let num_str: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    num_str.parse().ok()
}

/// Extract a string value for a given key from a JSON fragment.
fn extract_json_string<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let pattern = format!("\"{}\":\"", key);
    let pos = json.find(&pattern)? + pattern.len();
    let end = json[pos..].find('"')? + pos;
    Some(&json[pos..end])
}

#[cfg(test)]
mod frontier_tests {
    use super::*;

    #[test]
    fn test_mixed_frontier_roundtrip() {
        let mut frontier = MixedFrontier::new();
        frontier.set(
            "ducklake_catalog".to_string(),
            SourceFrontier::DuckLakeSnapshot(42),
        );
        frontier.set(
            "pg_source".to_string(),
            SourceFrontier::Opaque(vec![0xDE, 0xAD, 0xBE, 0xEF]),
        );
        frontier.set("internal".to_string(), SourceFrontier::SequenceNumber(100));

        let json = frontier.to_json();
        let parsed = MixedFrontier::from_json(&json).unwrap();
        assert_eq!(frontier, parsed);
    }

    #[test]
    fn test_ducklake_snapshot_extraction() {
        let mut frontier = MixedFrontier::new();
        frontier.set("catalog".to_string(), SourceFrontier::DuckLakeSnapshot(99));
        frontier.set("pg".to_string(), SourceFrontier::Opaque(vec![1, 2, 3]));

        assert_eq!(frontier.ducklake_snapshot(), Some(99));
    }

    #[test]
    fn test_empty_frontier() {
        let frontier = MixedFrontier::new();
        let json = frontier.to_json();
        assert_eq!(json, "{}");
        let parsed = MixedFrontier::from_json(&json).unwrap();
        assert_eq!(parsed.sources.len(), 0);
    }

    #[test]
    fn test_opaque_base64_encoding() {
        let mut frontier = MixedFrontier::new();
        let bytes = vec![0x00, 0xFF, 0x42, 0x13, 0x37];
        frontier.set("wal".to_string(), SourceFrontier::Opaque(bytes.clone()));

        let json = frontier.to_json();
        assert!(json.contains("opaque"));

        let parsed = MixedFrontier::from_json(&json).unwrap();
        assert_eq!(parsed.get("wal"), Some(&SourceFrontier::Opaque(bytes)));
    }
}
