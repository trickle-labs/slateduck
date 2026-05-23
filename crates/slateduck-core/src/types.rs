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
    /// Timestamp types with optional timezone.
    Timestamp { with_timezone: bool },
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
    /// Unknown / unsupported type.
    Unknown(String),
}

/// Error returned when type-aware comparison fails.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TypeCompareError {
    #[error("unsupported type for comparison: {0} (SQLSTATE 0A000)")]
    UnsupportedType(String),
    #[error("failed to parse value '{value}' as {type_name}")]
    ParseError { value: String, type_name: String },
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
            Ok(compare_floats(l, r))
        }
        DuckLakeType::Timestamp { .. } | DuckLakeType::Date | DuckLakeType::Time { .. } => {
            // Timestamps, dates, and times are compared lexicographically in ISO format
            Ok(left.cmp(right))
        }
        DuckLakeType::Varchar | DuckLakeType::Uuid => Ok(left.cmp(right)),
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

/// Check if a file can be pruned based on its column stats.
///
/// Given a predicate value, min and max stats, and `contains_nan`:
/// - If value < min → prune (for equality/range queries)
/// - If value > max → prune
/// - For float types: if contains_nan and the query involves NaN, keep
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

    // If min > predicate → prune
    if let Some(min) = min_value {
        if !min.is_empty() {
            let cmp = type_aware_compare(predicate_value, min, col_type)?;
            if cmp == Ordering::Less {
                return Ok(PruneResult::Prune);
            }
        }
    }

    // If max < predicate → prune
    if let Some(max) = max_value {
        if !max.is_empty() {
            let cmp = type_aware_compare(predicate_value, max, col_type)?;
            if cmp == Ordering::Greater {
                return Ok(PruneResult::Prune);
            }
        }
    }

    Ok(PruneResult::Keep)
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

/// Compare two f64 values, with NaN handled separately.
fn compare_floats(a: f64, b: f64) -> Ordering {
    a.partial_cmp(&b).unwrap_or(Ordering::Equal)
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
