//! DuckLake type system and type-aware column statistics comparison.
//!
//! `prune_files()` uses these for type-aware comparisons.
//! Unknown types fail closed (SQLSTATE 0A000) rather than guessing.

use std::cmp::Ordering;

/// DuckLake column types for type-aware statistics comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DuckLakeType {
    /// Signed integer types (TINYINT, SMALLINT, INTEGER, BIGINT, HUGEINT).
    Integer { signed: bool, width_bits: u16 },
    /// Decimal / numeric with precision and scale.
    Decimal { precision: u8, scale: u8 },
    /// IEEE floating-point (FLOAT, DOUBLE).
    Float { width_bits: u16 },
    /// Timestamp types with optional timezone and explicit precision.
    /// precision: 0=seconds, 3=milliseconds, 6=microseconds, 9=nanoseconds.
    Timestamp { with_timezone: bool, precision: u8 },
    /// Date type.
    Date,
    /// Time type with optional timezone.
    Time { with_timezone: bool },
    /// Interval type.
    Interval,
    /// String / VARCHAR / TEXT.
    Varchar,
    /// BLOB / BYTEA.
    Blob,
    /// Boolean.
    Boolean,
    /// UUID.
    Uuid,
    /// JSON type.
    Json,
    /// Variant (semi-structured) type.
    Variant,
    /// Geometry / spatial type.
    Geometry,
    /// List type with element type.
    List(Box<DuckLakeType>),
    /// Struct type with named fields.
    Struct(Vec<(String, DuckLakeType)>),
    /// Map type with key and value types.
    Map {
        key: Box<DuckLakeType>,
        value: Box<DuckLakeType>,
    },
    /// Unknown / unsupported type.
    Unknown(String),
}

impl DuckLakeType {
    /// Parse a DuckLake type string into a typed enum variant.
    ///
    /// Handles all spec primitive types and the three nested type forms
    /// (`list<T>`, `struct<f:T,...>`, `map<K,V>`).
    pub fn parse(s: &str) -> DuckLakeType {
        let s = s.trim();
        let lower = s.to_ascii_lowercase();
        let lower = lower.trim();

        // Nested: list<T>
        if lower.starts_with("list<") && lower.ends_with('>') {
            let inner = &s[5..s.len() - 1];
            return DuckLakeType::List(Box::new(DuckLakeType::parse(inner)));
        }

        // Nested: struct<f:T,...>
        if lower.starts_with("struct<") && lower.ends_with('>') {
            let inner = &s[7..s.len() - 1];
            let mut fields = Vec::new();
            for part in split_top_level(inner, ',') {
                if let Some(colon) = part.find(':') {
                    let fname = part[..colon].trim().to_string();
                    let ftype = DuckLakeType::parse(part[colon + 1..].trim());
                    fields.push((fname, ftype));
                }
            }
            return DuckLakeType::Struct(fields);
        }

        // Nested: map<K,V>
        if lower.starts_with("map<") && lower.ends_with('>') {
            let inner = &s[4..s.len() - 1];
            let parts: Vec<&str> = split_top_level(inner, ',');
            if parts.len() == 2 {
                let key = Box::new(DuckLakeType::parse(parts[0].trim()));
                let value = Box::new(DuckLakeType::parse(parts[1].trim()));
                return DuckLakeType::Map { key, value };
            }
        }

        // Decimal: decimal(P,S) or numeric(P,S)
        if (lower.starts_with("decimal(") || lower.starts_with("numeric(")) && lower.ends_with(')')
        {
            let start = lower.find('(').unwrap_or(0) + 1;
            let inner = &lower[start..lower.len() - 1];
            let parts: Vec<&str> = inner.split(',').collect();
            if parts.len() == 2 {
                let precision = parts[0].trim().parse::<u8>().unwrap_or(18);
                let scale = parts[1].trim().parse::<u8>().unwrap_or(0);
                return DuckLakeType::Decimal { precision, scale };
            }
            return DuckLakeType::Decimal {
                precision: 18,
                scale: 0,
            };
        }

        match lower {
            // Signed integers
            "int8" | "tinyint" => DuckLakeType::Integer {
                signed: true,
                width_bits: 8,
            },
            "int16" | "smallint" | "short" => DuckLakeType::Integer {
                signed: true,
                width_bits: 16,
            },
            "int32" | "int" | "integer" | "signed" => DuckLakeType::Integer {
                signed: true,
                width_bits: 32,
            },
            "int64" | "bigint" | "long" => DuckLakeType::Integer {
                signed: true,
                width_bits: 64,
            },
            "int128" | "hugeint" => DuckLakeType::Integer {
                signed: true,
                width_bits: 128,
            },
            // Unsigned integers
            "uint8" | "utinyint" => DuckLakeType::Integer {
                signed: false,
                width_bits: 8,
            },
            "uint16" | "usmallint" => DuckLakeType::Integer {
                signed: false,
                width_bits: 16,
            },
            "uint32" | "uinteger" => DuckLakeType::Integer {
                signed: false,
                width_bits: 32,
            },
            "uint64" | "ubigint" => DuckLakeType::Integer {
                signed: false,
                width_bits: 64,
            },
            "uint128" | "uhugeint" => DuckLakeType::Integer {
                signed: false,
                width_bits: 128,
            },
            // Floats
            "float" | "float32" | "real" => DuckLakeType::Float { width_bits: 32 },
            "double" | "float64" | "float8" => DuckLakeType::Float { width_bits: 64 },
            // Timestamp variants
            "timestamp" | "datetime" => DuckLakeType::Timestamp {
                with_timezone: false,
                precision: 6,
            },
            "timestamp with time zone" | "timestamptz" | "timestamp_tz" => {
                DuckLakeType::Timestamp {
                    with_timezone: true,
                    precision: 6,
                }
            }
            "timestamp_s" => DuckLakeType::Timestamp {
                with_timezone: false,
                precision: 0,
            },
            "timestamp_ms" => DuckLakeType::Timestamp {
                with_timezone: false,
                precision: 3,
            },
            "timestamp_us" => DuckLakeType::Timestamp {
                with_timezone: false,
                precision: 6,
            },
            "timestamp_ns" => DuckLakeType::Timestamp {
                with_timezone: false,
                precision: 9,
            },
            // Date / Time
            "date" | "date32" => DuckLakeType::Date,
            "time" | "time without time zone" => DuckLakeType::Time {
                with_timezone: false,
            },
            "time with time zone" | "timetz" | "time_tz" => DuckLakeType::Time {
                with_timezone: true,
            },
            "interval" => DuckLakeType::Interval,
            // String types
            "varchar" | "text" | "string" | "char" | "bpchar" | "name" => DuckLakeType::Varchar,
            // Binary
            "blob" | "bytea" | "binary" | "varbinary" => DuckLakeType::Blob,
            // Boolean
            "boolean" | "bool" | "logical" => DuckLakeType::Boolean,
            // UUID
            "uuid" => DuckLakeType::Uuid,
            // JSON
            "json" => DuckLakeType::Json,
            // Variant / semi-structured
            "variant" => DuckLakeType::Variant,
            // Geometry
            "geometry" | "wkb_blob" => DuckLakeType::Geometry,
            // Decimal shorthand
            "decimal" | "numeric" => DuckLakeType::Decimal {
                precision: 18,
                scale: 3,
            },
            // Unknown
            other => DuckLakeType::Unknown(other.to_string()),
        }
    }
}

/// Error returned when type-aware comparison fails.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TypeCompareError {
    #[error("unsupported type for comparison: {0} (SQLSTATE 0A000)")]
    UnsupportedType(String),
    #[error("failed to parse value '{value}' as {type_name}")]
    ParseError { value: String, type_name: String },
    #[error("NaN comparison is undefined; treated as keep-file (SQLSTATE 22023)")]
    NanComparison,
}

/// Result of a pruning comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PruneResult {
    /// File can be pruned (does not contain matching data).
    Prune,
    /// File cannot be pruned (may contain matching data).
    Keep,
}

/// Compare two statistic values according to the given type.
/// Returns `Ordering` or an error for unknown types.
pub fn type_aware_compare(
    left: &str,
    right: &str,
    col_type: &DuckLakeType,
) -> Result<Ordering, TypeCompareError> {
    match col_type {
        DuckLakeType::Integer { signed, .. } => {
            if *signed {
                let l: i128 = left.parse().map_err(|_| TypeCompareError::ParseError {
                    value: left.to_string(),
                    type_name: "signed integer".to_string(),
                })?;
                let r: i128 = right.parse().map_err(|_| TypeCompareError::ParseError {
                    value: right.to_string(),
                    type_name: "signed integer".to_string(),
                })?;
                Ok(l.cmp(&r))
            } else {
                let l: u128 = left.parse().map_err(|_| TypeCompareError::ParseError {
                    value: left.to_string(),
                    type_name: "unsigned integer".to_string(),
                })?;
                let r: u128 = right.parse().map_err(|_| TypeCompareError::ParseError {
                    value: right.to_string(),
                    type_name: "unsigned integer".to_string(),
                })?;
                Ok(l.cmp(&r))
            }
        }
        DuckLakeType::Decimal { .. } => {
            // Parse as rational: split on '.', normalize
            let l = parse_decimal(left).map_err(|_| TypeCompareError::ParseError {
                value: left.to_string(),
                type_name: "decimal".to_string(),
            })?;
            let r = parse_decimal(right).map_err(|_| TypeCompareError::ParseError {
                value: right.to_string(),
                type_name: "decimal".to_string(),
            })?;
            Ok(l.cmp(&r))
        }
        DuckLakeType::Float { .. } => {
            let l = parse_float(left).map_err(|_| TypeCompareError::ParseError {
                value: left.to_string(),
                type_name: "float".to_string(),
            })?;
            let r = parse_float(right).map_err(|_| TypeCompareError::ParseError {
                value: right.to_string(),
                type_name: "float".to_string(),
            })?;
            Ok(compare_floats(l, r)?)
        }
        DuckLakeType::Timestamp { .. } | DuckLakeType::Date | DuckLakeType::Time { .. } => {
            // Timestamps, dates, and times are compared lexicographically in ISO format
            Ok(left.cmp(right))
        }
        DuckLakeType::Varchar | DuckLakeType::Uuid | DuckLakeType::Json => Ok(left.cmp(right)),
        // Variant, Geometry, and nested types: no meaningful min/max comparison; keep file.
        DuckLakeType::Variant
        | DuckLakeType::Geometry
        | DuckLakeType::List(_)
        | DuckLakeType::Struct(_)
        | DuckLakeType::Map { .. } => Ok(Ordering::Equal),
        DuckLakeType::Boolean => {
            let l = parse_bool(left).map_err(|_| TypeCompareError::ParseError {
                value: left.to_string(),
                type_name: "boolean".to_string(),
            })?;
            let r = parse_bool(right).map_err(|_| TypeCompareError::ParseError {
                value: right.to_string(),
                type_name: "boolean".to_string(),
            })?;
            Ok(l.cmp(&r))
        }
        DuckLakeType::Blob | DuckLakeType::Interval => {
            // Blob and Interval: lexicographic on string representation
            Ok(left.cmp(right))
        }
        DuckLakeType::Unknown(name) => Err(TypeCompareError::UnsupportedType(name.clone())),
    }
}

/// Split a string by `sep` respecting angle-bracket nesting depth.
fn split_top_level(s: &str, sep: char) -> Vec<&str> {
    let mut result = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (i, c) in s.char_indices() {
        match c {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            _ if c == sep && depth == 0 => {
                result.push(&s[start..i]);
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    result.push(&s[start..]);
    result
}

/// Check if a file can be pruned based on its column stats.
///
/// Given a predicate value, min and max stats, and `contains_nan`:
/// - If value < min → prune (for equality/range queries)
/// - If value > max → prune
/// - For float types: if contains_nan and the query involves NaN, keep
/// - Variant, Geometry, and nested types always keep (no min/max pruning)
pub fn prune_file(
    predicate_value: &str,
    min_value: Option<&str>,
    max_value: Option<&str>,
    contains_nan: bool,
    col_type: &DuckLakeType,
) -> Result<PruneResult, TypeCompareError> {
    // Unknown types fail closed
    if matches!(col_type, DuckLakeType::Unknown(_)) {
        return Err(TypeCompareError::UnsupportedType(format!("{col_type:?}")));
    }

    // Variant, Geometry, and nested types: no pruning possible, always keep
    if matches!(
        col_type,
        DuckLakeType::Variant
            | DuckLakeType::Geometry
            | DuckLakeType::Json
            | DuckLakeType::List(_)
            | DuckLakeType::Struct(_)
            | DuckLakeType::Map { .. }
    ) {
        return Ok(PruneResult::Keep);
    }

    // Handle NaN for float types
    if matches!(col_type, DuckLakeType::Float { .. })
        && (predicate_value == "NaN" || predicate_value == "nan")
    {
        return Ok(if contains_nan {
            PruneResult::Keep
        } else {
            PruneResult::Prune
        });
    }

    // If min > predicate → prune (NaN comparison fails closed: keep the file)
    if let Some(min) = min_value {
        if !min.is_empty() {
            match type_aware_compare(predicate_value, min, col_type) {
                Err(TypeCompareError::NanComparison) => return Ok(PruneResult::Keep),
                Err(e) => return Err(e),
                Ok(cmp) => {
                    if cmp == Ordering::Less {
                        return Ok(PruneResult::Prune);
                    }
                }
            }
        }
    }

    // If max < predicate → prune (NaN comparison fails closed: keep the file)
    if let Some(max) = max_value {
        if !max.is_empty() {
            match type_aware_compare(predicate_value, max, col_type) {
                Err(TypeCompareError::NanComparison) => return Ok(PruneResult::Keep),
                Err(e) => return Err(e),
                Ok(cmp) => {
                    if cmp == Ordering::Greater {
                        return Ok(PruneResult::Prune);
                    }
                }
            }
        }
    }

    Ok(PruneResult::Keep)
}

// ─── Geometry Extra Stats ──────────────────────────────────────────────────

/// Geometry bounding box and type metadata serialized into `extra_stats` JSON.
///
/// Stored as a JSON blob in the `extra_stats` field of `FileColumnStatsRow`
/// for columns with `column_type = 'geometry'`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GeometryExtraStats {
    /// Minimum X coordinate.
    pub min_x: Option<f64>,
    /// Maximum X coordinate.
    pub max_x: Option<f64>,
    /// Minimum Y coordinate.
    pub min_y: Option<f64>,
    /// Maximum Y coordinate.
    pub max_y: Option<f64>,
    /// Minimum Z coordinate (for 3D geometries).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_z: Option<f64>,
    /// Maximum Z coordinate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_z: Option<f64>,
    /// Minimum M coordinate (for measured geometries).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_m: Option<f64>,
    /// Maximum M coordinate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_m: Option<f64>,
    /// Geometry type string (e.g. "POLYGON", "MULTILINESTRING").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub geometry_type: Option<String>,
    /// Spatial Reference System Identifier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub srid: Option<i32>,
}

impl GeometryExtraStats {
    /// Serialize to JSON for storage in `extra_stats`.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Deserialize from a JSON `extra_stats` string.
    pub fn from_json(s: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(s)
    }

    /// Validate that the bounding box values are consistent (min ≤ max).
    /// Returns `Err` with a description if validation fails.
    pub fn validate(&self) -> Result<(), String> {
        if let (Some(min_x), Some(max_x)) = (self.min_x, self.max_x) {
            if min_x > max_x {
                return Err(format!("min_x ({min_x}) > max_x ({max_x})"));
            }
        }
        if let (Some(min_y), Some(max_y)) = (self.min_y, self.max_y) {
            if min_y > max_y {
                return Err(format!("min_y ({min_y}) > max_y ({max_y})"));
            }
        }
        if let (Some(min_z), Some(max_z)) = (self.min_z, self.max_z) {
            if min_z > max_z {
                return Err(format!("min_z ({min_z}) > max_z ({max_z})"));
            }
        }
        if let (Some(min_m), Some(max_m)) = (self.min_m, self.max_m) {
            if min_m > max_m {
                return Err(format!("min_m ({min_m}) > max_m ({max_m})"));
            }
        }
        Ok(())
    }

    /// Check if a spatial point (x, y) is within the bounding box.
    ///
    /// Returns `PruneResult::Prune` if the point is definitely outside the box,
    /// `PruneResult::Keep` otherwise.
    pub fn prune_by_point(&self, x: f64, y: f64) -> PruneResult {
        if let (Some(min_x), Some(max_x)) = (self.min_x, self.max_x) {
            if x < min_x || x > max_x {
                return PruneResult::Prune;
            }
        }
        if let (Some(min_y), Some(max_y)) = (self.min_y, self.max_y) {
            if y < min_y || y > max_y {
                return PruneResult::Prune;
            }
        }
        PruneResult::Keep
    }
}

/// Validate that an `extra_stats` JSON string is well-formed.
///
/// Returns `Ok(())` if the string is valid JSON or is `None`,
/// `Err` if the string is present but not valid JSON.
pub fn validate_extra_stats(extra_stats: Option<&str>) -> Result<(), String> {
    if let Some(s) = extra_stats {
        if !s.is_empty() {
            serde_json::from_str::<serde_json::Value>(s)
                .map_err(|e| format!("extra_stats is not valid JSON: {e}"))?;
        }
    }
    Ok(())
}

// ─── Internal Helpers ──────────────────────────────────────────────────────

/// Parse a decimal string to a comparable integer representation.
/// We multiply by 10^scale to compare as integers.
fn parse_decimal(s: &str) -> Result<i128, ()> {
    let s = s.trim();
    let negative = s.starts_with('-');
    let s = s.trim_start_matches('-').trim_start_matches('+');

    let (integer_part, decimal_part) = if let Some(dot_pos) = s.find('.') {
        (&s[..dot_pos], &s[dot_pos + 1..])
    } else {
        (s, "")
    };

    // Normalize to 18 decimal places for comparison
    let scale = 18usize;
    let padded_decimal = format!("{:0<width$}", decimal_part, width = scale);
    let combined = format!("{}{}", integer_part, &padded_decimal[..scale]);
    let val: i128 = combined.parse().map_err(|_| ())?;
    Ok(if negative { -val } else { val })
}

/// Parse a float string, handling inf/-inf/NaN.
fn parse_float(s: &str) -> Result<f64, ()> {
    match s.to_lowercase().as_str() {
        "inf" | "infinity" | "+inf" | "+infinity" => Ok(f64::INFINITY),
        "-inf" | "-infinity" => Ok(f64::NEG_INFINITY),
        "nan" => Ok(f64::NAN),
        _ => s.parse::<f64>().map_err(|_| ()),
    }
}

/// Compare two f64 values, returning an error if either operand is NaN.
/// Callers should treat NanComparison as "keep the file" (fail-closed).
fn compare_floats(a: f64, b: f64) -> Result<Ordering, TypeCompareError> {
    a.partial_cmp(&b).ok_or(TypeCompareError::NanComparison)
}

/// Parse a boolean string.
fn parse_bool(s: &str) -> Result<bool, ()> {
    match s.to_lowercase().as_str() {
        "true" | "t" | "1" | "yes" => Ok(true),
        "false" | "f" | "0" | "no" => Ok(false),
        _ => Err(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_comparison_signed() {
        let t = DuckLakeType::Integer {
            signed: true,
            width_bits: 64,
        };
        assert_eq!(type_aware_compare("-10", "5", &t).unwrap(), Ordering::Less);
        assert_eq!(
            type_aware_compare("100", "99", &t).unwrap(),
            Ordering::Greater
        );
    }

    #[test]
    fn integer_comparison_unsigned() {
        let t = DuckLakeType::Integer {
            signed: false,
            width_bits: 64,
        };
        assert_eq!(
            type_aware_compare("10", "5", &t).unwrap(),
            Ordering::Greater
        );
    }

    #[test]
    fn decimal_comparison() {
        let t = DuckLakeType::Decimal {
            precision: 10,
            scale: 2,
        };
        assert_eq!(
            type_aware_compare("1.5", "1.50", &t).unwrap(),
            Ordering::Equal
        );
        assert_eq!(
            type_aware_compare("1.49", "1.5", &t).unwrap(),
            Ordering::Less
        );
    }

    #[test]
    fn float_infinity() {
        let t = DuckLakeType::Float { width_bits: 64 };
        assert_eq!(
            type_aware_compare("inf", "100.0", &t).unwrap(),
            Ordering::Greater
        );
        assert_eq!(
            type_aware_compare("-inf", "-100.0", &t).unwrap(),
            Ordering::Less
        );
    }

    #[test]
    fn unknown_type_fails_closed() {
        let t = DuckLakeType::Unknown("GEOMETRY".to_string());
        assert!(type_aware_compare("a", "b", &t).is_err());
    }

    #[test]
    fn prune_file_basic() {
        let t = DuckLakeType::Integer {
            signed: true,
            width_bits: 32,
        };
        // predicate=50, min=100, max=200 → prune (50 < 100)
        assert_eq!(
            prune_file("50", Some("100"), Some("200"), false, &t).unwrap(),
            PruneResult::Prune
        );
        // predicate=150, min=100, max=200 → keep
        assert_eq!(
            prune_file("150", Some("100"), Some("200"), false, &t).unwrap(),
            PruneResult::Keep
        );
        // predicate=250, min=100, max=200 → prune (250 > 200)
        assert_eq!(
            prune_file("250", Some("100"), Some("200"), false, &t).unwrap(),
            PruneResult::Prune
        );
    }

    #[test]
    fn prune_file_nan() {
        let t = DuckLakeType::Float { width_bits: 64 };
        assert_eq!(
            prune_file("NaN", Some("1.0"), Some("10.0"), true, &t).unwrap(),
            PruneResult::Keep
        );
        assert_eq!(
            prune_file("NaN", Some("1.0"), Some("10.0"), false, &t).unwrap(),
            PruneResult::Prune
        );
    }

    #[test]
    fn prune_file_unknown_type_error() {
        let t = DuckLakeType::Unknown("JSONB".to_string());
        assert!(prune_file("x", Some("a"), Some("z"), false, &t).is_err());
    }
}
