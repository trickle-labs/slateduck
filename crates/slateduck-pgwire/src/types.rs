//! PostgreSQL type OIDs and text encoders/decoders.
//!
//! Implements the types observed in the Phase 0 wire corpus.

/// PostgreSQL type OIDs used by DuckLake.
pub mod oid {
    pub const BOOL: u32 = 16;
    pub const INT2: u32 = 21;
    pub const INT4: u32 = 23;
    pub const INT8: u32 = 20;
    pub const FLOAT4: u32 = 700;
    pub const FLOAT8: u32 = 701;
    pub const TEXT: u32 = 25;
    pub const VARCHAR: u32 = 1043;
    pub const TIMESTAMP: u32 = 1114;
    pub const TIMESTAMPTZ: u32 = 1184;
    pub const UUID: u32 = 2950;
    pub const JSON: u32 = 114;
    pub const JSONB: u32 = 3802;
}

/// Known PG type name to OID mapping for `pg_catalog.pg_type` queries.
pub const PG_TYPE_MAP: &[(&str, u32)] = &[
    ("bool", oid::BOOL),
    ("int2", oid::INT2),
    ("int4", oid::INT4),
    ("int8", oid::INT8),
    ("float4", oid::FLOAT4),
    ("float8", oid::FLOAT8),
    ("text", oid::TEXT),
    ("varchar", oid::VARCHAR),
    ("timestamp", oid::TIMESTAMP),
    ("timestamptz", oid::TIMESTAMPTZ),
    ("uuid", oid::UUID),
    ("json", oid::JSON),
    ("jsonb", oid::JSONB),
];

/// Get the OID for a type name, or None if unknown.
pub fn type_name_to_oid(name: &str) -> Option<u32> {
    PG_TYPE_MAP
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, oid)| *oid)
}

/// Get the type name for an OID, or None if unknown.
pub fn oid_to_type_name(oid_val: u32) -> Option<&'static str> {
    PG_TYPE_MAP
        .iter()
        .find(|(_, o)| *o == oid_val)
        .map(|(n, _)| *n)
}
