//! Table tag bytes and key-shape metadata for all 28 DuckLake v1.0 tables
//! plus SlateDuck system namespaces.
//!
//! This module is the **single source of truth** for tag allocation.
//! Every tag byte is allocated up front; unknown tags produce an explicit error.

/// Implementation status of a table tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagStatus {
    /// Fully implemented in this release.
    Live,
    /// Tag allocated but implementation deferred to a later phase.
    Deferred(u8),
    /// Tag allocated but not yet implemented.
    Unimplemented,
}

/// MVCC behavior for a table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MvccBehavior {
    /// Row is versioned with `begin_snapshot`/`end_snapshot`.
    Versioned,
    /// Row is written once and never updated (append-only).
    AppendOnly,
    /// Row is a mutable singleton (e.g., counters, system keys).
    MutableSingleton,
    /// Row has custom MVCC semantics (e.g., inlined data).
    Custom,
}

/// Whether a unique-guard key is needed for this table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UniqueGuard {
    /// No unique-guard key needed; key structure enforces uniqueness.
    NotNeeded,
    /// Unique-guard key required under `0xFE` prefix.
    Required,
}

/// Descriptor for a single table tag.
#[derive(Debug, Clone)]
pub struct TagDescriptor {
    /// The tag byte (first byte of key).
    pub tag: u8,
    /// Human-readable table name.
    pub name: &'static str,
    /// Key shape description.
    pub key_shape: &'static str,
    /// MVCC behavior.
    pub mvcc: MvccBehavior,
    /// Whether a unique-guard key is needed.
    pub unique_guard: UniqueGuard,
    /// Implementation status.
    pub status: TagStatus,
}

// ─── DuckLake Catalog Table Tags ───────────────────────────────────────────

pub const TAG_METADATA: u8 = 0x01;
pub const TAG_SNAPSHOT: u8 = 0x02;
pub const TAG_SNAPSHOT_CHANGES: u8 = 0x03;
pub const TAG_SCHEMA: u8 = 0x04;
pub const TAG_TABLE: u8 = 0x05;
pub const TAG_COLUMN: u8 = 0x06;
pub const TAG_VIEW: u8 = 0x07;
pub const TAG_MACRO: u8 = 0x08;
pub const TAG_MACRO_IMPL: u8 = 0x09;
pub const TAG_MACRO_PARAMETERS: u8 = 0x0A;
pub const TAG_DATA_FILE: u8 = 0x0B;
pub const TAG_DELETE_FILE: u8 = 0x0C;
pub const TAG_FILES_SCHEDULED_FOR_DELETION: u8 = 0x0D;
pub const TAG_INLINED_DATA_TABLES: u8 = 0x0E;
pub const TAG_COLUMN_MAPPING: u8 = 0x0F;
pub const TAG_NAME_MAPPING: u8 = 0x10;
pub const TAG_TABLE_STATS: u8 = 0x11;
pub const TAG_TABLE_COLUMN_STATS: u8 = 0x12;
pub const TAG_FILE_COLUMN_STATS: u8 = 0x13;
pub const TAG_FILE_VARIANT_STATS: u8 = 0x14;
pub const TAG_PARTITION_INFO: u8 = 0x15;
pub const TAG_PARTITION_COLUMN: u8 = 0x16;
pub const TAG_FILE_PARTITION_VALUE: u8 = 0x17;
pub const TAG_SORT_INFO: u8 = 0x18;
pub const TAG_SORT_EXPRESSION: u8 = 0x19;
pub const TAG_TAG: u8 = 0x1A;
pub const TAG_COLUMN_TAG: u8 = 0x1B;
pub const TAG_SCHEMA_VERSIONS: u8 = 0x1C;

// ─── SlateDuck Internal Tags ───────────────────────────────────────────────

/// Dynamic inlined rows (subtype 0x01 = insert, 0x02 = delete marker).
pub const TAG_INLINED_ROWS: u8 = 0xFD;
/// SlateDuck counters (next_snapshot_id, next_catalog_id, etc.).
pub const TAG_COUNTERS: u8 = 0xFE;
/// SlateDuck system keys (writer epoch, endpoint, retain-from, catalog-format-version).
pub const TAG_SYSTEM: u8 = 0xFF;

// ─── Inlined Row Subtypes ──────────────────────────────────────────────────

pub const INLINED_SUBTYPE_INSERT: u8 = 0x01;
pub const INLINED_SUBTYPE_DELETE: u8 = 0x02;

// ─── Counter IDs ───────────────────────────────────────────────────────────

pub const COUNTER_NEXT_SNAPSHOT_ID: u8 = 0x01;
pub const COUNTER_NEXT_CATALOG_ID: u8 = 0x02;
pub const COUNTER_NEXT_FILE_ID: u8 = 0x03;
/// Per-table column ID counter: `0xFE | 0x10 | table_id(u64 BE)`.
pub const COUNTER_NEXT_COLUMN_ID_PREFIX: u8 = 0x10;

// ─── System Key Suffixes ───────────────────────────────────────────────────

pub const SYSTEM_WRITER_EPOCH: &[u8] = b"writer-epoch";
pub const SYSTEM_ENDPOINT: &[u8] = b"endpoint";
pub const SYSTEM_RETAIN_FROM: &[u8] = b"retain-from";
pub const SYSTEM_CATALOG_FORMAT_VERSION: &[u8] = b"catalog-format-version";
pub const SYSTEM_EXCISED_PREFIX: &[u8] = b"excised";

/// Current catalog format version. Mismatch on open → refuse.
pub const CATALOG_FORMAT_VERSION: u32 = 1;

/// Complete registry of all tag descriptors.
pub static ALL_TAGS: &[TagDescriptor] = &[
    TagDescriptor {
        tag: TAG_METADATA,
        name: "ducklake_metadata",
        key_shape: "scope_enum | scope_id | length-prefixed UTF-8 key",
        mvcc: MvccBehavior::MutableSingleton,
        unique_guard: UniqueGuard::NotNeeded,
        status: TagStatus::Live,
    },
    TagDescriptor {
        tag: TAG_SNAPSHOT,
        name: "ducklake_snapshot",
        key_shape: "snapshot_id",
        mvcc: MvccBehavior::AppendOnly,
        unique_guard: UniqueGuard::NotNeeded,
        status: TagStatus::Live,
    },
    TagDescriptor {
        tag: TAG_SNAPSHOT_CHANGES,
        name: "ducklake_snapshot_changes",
        key_shape: "snapshot_id",
        mvcc: MvccBehavior::AppendOnly,
        unique_guard: UniqueGuard::NotNeeded,
        status: TagStatus::Live,
    },
    TagDescriptor {
        tag: TAG_SCHEMA,
        name: "ducklake_schema",
        key_shape: "schema_id",
        mvcc: MvccBehavior::Versioned,
        unique_guard: UniqueGuard::Required,
        status: TagStatus::Live,
    },
    TagDescriptor {
        tag: TAG_TABLE,
        name: "ducklake_table",
        key_shape: "schema_id | table_id | begin_snapshot",
        mvcc: MvccBehavior::Versioned,
        unique_guard: UniqueGuard::Required,
        status: TagStatus::Live,
    },
    TagDescriptor {
        tag: TAG_COLUMN,
        name: "ducklake_column",
        key_shape: "table_id | column_id | begin_snapshot",
        mvcc: MvccBehavior::Versioned,
        unique_guard: UniqueGuard::NotNeeded,
        status: TagStatus::Live,
    },
    TagDescriptor {
        tag: TAG_VIEW,
        name: "ducklake_view",
        key_shape: "schema_id | view_id | begin_snapshot",
        mvcc: MvccBehavior::Versioned,
        unique_guard: UniqueGuard::Required,
        status: TagStatus::Live,
    },
    TagDescriptor {
        tag: TAG_MACRO,
        name: "ducklake_macro",
        key_shape: "schema_id | macro_id | begin_snapshot",
        mvcc: MvccBehavior::Versioned,
        unique_guard: UniqueGuard::Required,
        status: TagStatus::Live,
    },
    TagDescriptor {
        tag: TAG_MACRO_IMPL,
        name: "ducklake_macro_impl",
        key_shape: "macro_id | impl_id",
        mvcc: MvccBehavior::AppendOnly,
        unique_guard: UniqueGuard::NotNeeded,
        status: TagStatus::Live,
    },
    TagDescriptor {
        tag: TAG_MACRO_PARAMETERS,
        name: "ducklake_macro_parameters",
        key_shape: "macro_id | impl_id | column_id",
        mvcc: MvccBehavior::AppendOnly,
        unique_guard: UniqueGuard::NotNeeded,
        status: TagStatus::Live,
    },
    TagDescriptor {
        tag: TAG_DATA_FILE,
        name: "ducklake_data_file",
        key_shape: "table_id | data_file_id",
        mvcc: MvccBehavior::AppendOnly,
        unique_guard: UniqueGuard::NotNeeded,
        status: TagStatus::Live,
    },
    TagDescriptor {
        tag: TAG_DELETE_FILE,
        name: "ducklake_delete_file",
        key_shape: "data_file_id | delete_file_id",
        mvcc: MvccBehavior::AppendOnly,
        unique_guard: UniqueGuard::NotNeeded,
        status: TagStatus::Live,
    },
    TagDescriptor {
        tag: TAG_FILES_SCHEDULED_FOR_DELETION,
        name: "ducklake_files_scheduled_for_deletion",
        key_shape: "schedule_start | data_file_id",
        mvcc: MvccBehavior::AppendOnly,
        unique_guard: UniqueGuard::NotNeeded,
        status: TagStatus::Live,
    },
    TagDescriptor {
        tag: TAG_INLINED_DATA_TABLES,
        name: "ducklake_inlined_data_tables",
        key_shape: "table_id | schema_version",
        mvcc: MvccBehavior::AppendOnly,
        unique_guard: UniqueGuard::NotNeeded,
        status: TagStatus::Live,
    },
    TagDescriptor {
        tag: TAG_COLUMN_MAPPING,
        name: "ducklake_column_mapping",
        key_shape: "table_id | mapping_id",
        mvcc: MvccBehavior::AppendOnly,
        unique_guard: UniqueGuard::NotNeeded,
        status: TagStatus::Live,
    },
    TagDescriptor {
        tag: TAG_NAME_MAPPING,
        name: "ducklake_name_mapping",
        key_shape: "mapping_id | column_id | source_name_hash",
        mvcc: MvccBehavior::AppendOnly,
        unique_guard: UniqueGuard::NotNeeded,
        status: TagStatus::Live,
    },
    TagDescriptor {
        tag: TAG_TABLE_STATS,
        name: "ducklake_table_stats",
        key_shape: "table_id",
        mvcc: MvccBehavior::MutableSingleton,
        unique_guard: UniqueGuard::NotNeeded,
        status: TagStatus::Live,
    },
    TagDescriptor {
        tag: TAG_TABLE_COLUMN_STATS,
        name: "ducklake_table_column_stats",
        key_shape: "table_id | column_id",
        mvcc: MvccBehavior::MutableSingleton,
        unique_guard: UniqueGuard::NotNeeded,
        status: TagStatus::Live,
    },
    TagDescriptor {
        tag: TAG_FILE_COLUMN_STATS,
        name: "ducklake_file_column_stats",
        key_shape: "table_id | column_id | data_file_id",
        mvcc: MvccBehavior::AppendOnly,
        unique_guard: UniqueGuard::NotNeeded,
        status: TagStatus::Live,
    },
    TagDescriptor {
        tag: TAG_FILE_VARIANT_STATS,
        name: "ducklake_file_variant_stats",
        key_shape: "table_id | column_id | variant_path_hash | data_file_id",
        mvcc: MvccBehavior::AppendOnly,
        unique_guard: UniqueGuard::NotNeeded,
        status: TagStatus::Live,
    },
    TagDescriptor {
        tag: TAG_PARTITION_INFO,
        name: "ducklake_partition_info",
        key_shape: "table_id | partition_id | begin_snapshot",
        mvcc: MvccBehavior::Versioned,
        unique_guard: UniqueGuard::NotNeeded,
        status: TagStatus::Live,
    },
    TagDescriptor {
        tag: TAG_PARTITION_COLUMN,
        name: "ducklake_partition_column",
        key_shape: "partition_id | partition_key_index",
        mvcc: MvccBehavior::AppendOnly,
        unique_guard: UniqueGuard::NotNeeded,
        status: TagStatus::Live,
    },
    TagDescriptor {
        tag: TAG_FILE_PARTITION_VALUE,
        name: "ducklake_file_partition_value",
        key_shape: "table_id | partition_key_index | data_file_id",
        mvcc: MvccBehavior::AppendOnly,
        unique_guard: UniqueGuard::NotNeeded,
        status: TagStatus::Live,
    },
    TagDescriptor {
        tag: TAG_SORT_INFO,
        name: "ducklake_sort_info",
        key_shape: "table_id | sort_id | begin_snapshot",
        mvcc: MvccBehavior::Versioned,
        unique_guard: UniqueGuard::NotNeeded,
        status: TagStatus::Live,
    },
    TagDescriptor {
        tag: TAG_SORT_EXPRESSION,
        name: "ducklake_sort_expression",
        key_shape: "sort_id | sort_key_index",
        mvcc: MvccBehavior::AppendOnly,
        unique_guard: UniqueGuard::NotNeeded,
        status: TagStatus::Live,
    },
    TagDescriptor {
        tag: TAG_TAG,
        name: "ducklake_tag",
        key_shape: "object_id | tag_key | begin_snapshot",
        mvcc: MvccBehavior::Versioned,
        unique_guard: UniqueGuard::NotNeeded,
        status: TagStatus::Live,
    },
    TagDescriptor {
        tag: TAG_COLUMN_TAG,
        name: "ducklake_column_tag",
        key_shape: "table_id | column_id | tag_key | begin_snapshot",
        mvcc: MvccBehavior::Versioned,
        unique_guard: UniqueGuard::NotNeeded,
        status: TagStatus::Live,
    },
    TagDescriptor {
        tag: TAG_SCHEMA_VERSIONS,
        name: "ducklake_schema_versions",
        key_shape: "table_id | begin_snapshot",
        mvcc: MvccBehavior::AppendOnly,
        unique_guard: UniqueGuard::NotNeeded,
        status: TagStatus::Live,
    },
    // ─── SlateDuck Internal ───
    TagDescriptor {
        tag: TAG_INLINED_ROWS,
        name: "dynamic_inlined_rows",
        key_shape: "subtype | table_id | (schema_version | data_file_id) | row_id",
        mvcc: MvccBehavior::Custom,
        unique_guard: UniqueGuard::NotNeeded,
        status: TagStatus::Live,
    },
    TagDescriptor {
        tag: TAG_COUNTERS,
        name: "slateduck_counters",
        key_shape: "counter_id",
        mvcc: MvccBehavior::MutableSingleton,
        unique_guard: UniqueGuard::NotNeeded,
        status: TagStatus::Live,
    },
    TagDescriptor {
        tag: TAG_SYSTEM,
        name: "slateduck_system",
        key_shape: "system_key_suffix",
        mvcc: MvccBehavior::MutableSingleton,
        unique_guard: UniqueGuard::NotNeeded,
        status: TagStatus::Live,
    },
];

/// Look up a tag descriptor by tag byte. Returns `None` for unknown tags.
pub fn lookup_tag(tag: u8) -> Option<&'static TagDescriptor> {
    ALL_TAGS.iter().find(|d| d.tag == tag)
}

/// Returns true if the given tag byte is a known, allocated tag.
pub fn is_known_tag(tag: u8) -> bool {
    lookup_tag(tag).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_tags_unique() {
        let mut seen = std::collections::HashSet::new();
        for desc in ALL_TAGS {
            assert!(
                seen.insert(desc.tag),
                "Duplicate tag byte 0x{:02X} for {}",
                desc.tag,
                desc.name
            );
        }
    }

    #[test]
    fn all_ducklake_tags_allocated() {
        // 28 DuckLake tables (0x01..=0x1C) + 3 system tags (0xFD, 0xFE, 0xFF)
        assert_eq!(ALL_TAGS.len(), 31);
    }

    #[test]
    fn lookup_known_tag() {
        let desc = lookup_tag(TAG_METADATA).unwrap();
        assert_eq!(desc.name, "ducklake_metadata");
    }

    #[test]
    fn lookup_unknown_tag() {
        assert!(lookup_tag(0x50).is_none());
    }

    #[test]
    fn tag_ordering_ducklake_before_system() {
        // Use runtime values to avoid clippy::assertions_on_constants
        let schema_versions = TAG_SCHEMA_VERSIONS;
        let inlined_rows = TAG_INLINED_ROWS;
        let counters = TAG_COUNTERS;
        let system = TAG_SYSTEM;
        assert!(schema_versions < inlined_rows);
        assert!(inlined_rows < counters);
        assert!(counters < system);
    }
}
