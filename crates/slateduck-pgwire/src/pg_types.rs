//! PostgreSQL type OID mappings for the DuckLake wire corpus.
//!
//! All types observed in the Phase 0 corpus are supported in text format.

use postgres_types::Type;

/// PostgreSQL type OID constants observed in the DuckLake wire corpus.
pub const OID_BOOL: u32 = 16;
pub const OID_INT2: u32 = 21;
pub const OID_INT4: u32 = 23;
pub const OID_INT8: u32 = 20;
pub const OID_FLOAT4: u32 = 700;
pub const OID_FLOAT8: u32 = 701;
pub const OID_TEXT: u32 = 25;
pub const OID_VARCHAR: u32 = 1043;
pub const OID_TIMESTAMP: u32 = 1114;
pub const OID_TIMESTAMPTZ: u32 = 1184;
pub const OID_UUID: u32 = 2950;
pub const OID_JSON: u32 = 114;
pub const OID_JSONB: u32 = 3802;

/// Well-known pg_type entries that DuckDB queries during handshake.
pub struct PgTypeEntry {
    pub oid: u32,
    pub typname: &'static str,
}

/// All type entries needed for the DuckDB handshake.
pub static PG_TYPE_ENTRIES: &[PgTypeEntry] = &[
    PgTypeEntry {
        oid: OID_BOOL,
        typname: "bool",
    },
    PgTypeEntry {
        oid: OID_INT2,
        typname: "int2",
    },
    PgTypeEntry {
        oid: OID_INT4,
        typname: "int4",
    },
    PgTypeEntry {
        oid: OID_INT8,
        typname: "int8",
    },
    PgTypeEntry {
        oid: OID_FLOAT4,
        typname: "float4",
    },
    PgTypeEntry {
        oid: OID_FLOAT8,
        typname: "float8",
    },
    PgTypeEntry {
        oid: OID_TEXT,
        typname: "text",
    },
    PgTypeEntry {
        oid: OID_VARCHAR,
        typname: "varchar",
    },
    PgTypeEntry {
        oid: OID_TIMESTAMP,
        typname: "timestamp",
    },
    PgTypeEntry {
        oid: OID_TIMESTAMPTZ,
        typname: "timestamptz",
    },
    PgTypeEntry {
        oid: OID_UUID,
        typname: "uuid",
    },
    PgTypeEntry {
        oid: OID_JSON,
        typname: "json",
    },
    PgTypeEntry {
        oid: OID_JSONB,
        typname: "jsonb",
    },
];

/// Get the postgres Type for a given OID.
pub fn type_for_oid(oid: u32) -> Type {
    Type::from_oid(oid).unwrap_or(Type::TEXT)
}

/// Map a DuckLake column type string to a postgres Type.
pub fn ducklake_type_to_pg(type_name: &str) -> Type {
    match type_name.to_uppercase().as_str() {
        "BOOLEAN" | "BOOL" => Type::BOOL,
        "TINYINT" | "INT1" => Type::INT2,
        "SMALLINT" | "INT2" => Type::INT2,
        "INTEGER" | "INT" | "INT4" => Type::INT4,
        "BIGINT" | "INT8" => Type::INT8,
        "FLOAT" | "FLOAT4" | "REAL" => Type::FLOAT4,
        "DOUBLE" | "FLOAT8" => Type::FLOAT8,
        "VARCHAR" | "TEXT" | "STRING" => Type::TEXT,
        "TIMESTAMP" => Type::TIMESTAMP,
        "TIMESTAMP WITH TIME ZONE" | "TIMESTAMPTZ" => Type::TIMESTAMPTZ,
        "UUID" => Type::UUID,
        "JSON" => Type::JSON,
        "JSONB" => Type::JSONB,
        _ => Type::TEXT, // Default to text for unknown types
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_oids_have_types() {
        assert_eq!(type_for_oid(OID_BOOL), Type::BOOL);
        assert_eq!(type_for_oid(OID_INT4), Type::INT4);
        assert_eq!(type_for_oid(OID_INT8), Type::INT8);
        assert_eq!(type_for_oid(OID_TEXT), Type::TEXT);
        assert_eq!(type_for_oid(OID_TIMESTAMP), Type::TIMESTAMP);
        assert_eq!(type_for_oid(OID_UUID), Type::UUID);
    }

    #[test]
    fn ducklake_type_mapping() {
        assert_eq!(ducklake_type_to_pg("BIGINT"), Type::INT8);
        assert_eq!(ducklake_type_to_pg("VARCHAR"), Type::TEXT);
        assert_eq!(ducklake_type_to_pg("BOOLEAN"), Type::BOOL);
        assert_eq!(ducklake_type_to_pg("unknown_type"), Type::TEXT);
    }

    #[test]
    fn pg_type_entries_cover_corpus_types() {
        let required = [
            "bool",
            "int4",
            "int8",
            "text",
            "varchar",
            "timestamp",
            "uuid",
        ];
        for r in required {
            assert!(
                PG_TYPE_ENTRIES.iter().any(|e| e.typname == r),
                "missing pg_type entry: {r}"
            );
        }
    }
}
