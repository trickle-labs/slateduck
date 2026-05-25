//! Parameter value storage for prepared statements.

/// Holds parameter values for a prepared statement execution.
#[derive(Debug, Clone, Default)]
pub struct ParamValues {
    values: Vec<Option<String>>,
}

impl ParamValues {
    pub fn new(values: Vec<Option<String>>) -> Self {
        Self { values }
    }

    pub fn get(&self, idx: usize) -> Option<&str> {
        self.values.get(idx).and_then(|v| v.as_deref())
    }

    pub fn get_u64(&self, idx: usize) -> Result<u64, super::SqlDispatchError> {
        let val = self
            .get(idx)
            .ok_or(super::SqlDispatchError::MissingParam(idx + 1))?;
        val.parse::<u64>()
            .map_err(|_| super::SqlDispatchError::TypeMismatch {
                idx: idx + 1,
                expected: "u64",
                actual: val.to_string(),
            })
    }

    pub fn get_i64(&self, idx: usize) -> Result<i64, super::SqlDispatchError> {
        let val = self
            .get(idx)
            .ok_or(super::SqlDispatchError::MissingParam(idx + 1))?;
        val.parse::<i64>()
            .map_err(|_| super::SqlDispatchError::TypeMismatch {
                idx: idx + 1,
                expected: "i64",
                actual: val.to_string(),
            })
    }

    pub fn get_string(&self, idx: usize) -> Result<String, super::SqlDispatchError> {
        self.get(idx)
            .map(|s| s.to_string())
            .ok_or(super::SqlDispatchError::MissingParam(idx + 1))
    }

    pub fn get_optional_string(&self, idx: usize) -> Option<String> {
        self.get(idx).map(|s| s.to_string())
    }

    pub fn get_bool(&self, idx: usize) -> Result<bool, super::SqlDispatchError> {
        let val = self
            .get(idx)
            .ok_or(super::SqlDispatchError::MissingParam(idx + 1))?;
        match val {
            "t" | "true" | "1" | "TRUE" => Ok(true),
            "f" | "false" | "0" | "FALSE" => Ok(false),
            _ => Err(super::SqlDispatchError::TypeMismatch {
                idx: idx + 1,
                expected: "bool",
                actual: val.to_string(),
            }),
        }
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Serialize all parameter values as a JSON object with positional keys (`p0`, `p1`, …).
    ///
    /// Values are properly escaped via `serde_json`; this is safe for all Unicode and
    /// control characters including embedded quotes, backslashes, and newlines.
    pub fn to_json_string(&self) -> String {
        let mut map = serde_json::Map::new();
        for (i, v) in self.values.iter().enumerate() {
            if let Some(val) = v {
                let key = format!("p{i}");
                map.insert(key, serde_json::Value::String(val.clone()));
            }
        }
        serde_json::Value::Object(map).to_string()
    }

    /// Serialize parameter values as a JSON object using the provided column names.
    ///
    /// If `columns` is shorter than the parameter list, remaining params fall back to
    /// positional keys (`p{N}`). Properly escapes all values via `serde_json`.
    pub fn to_json_string_with_columns(&self, columns: &[String]) -> String {
        let mut map = serde_json::Map::new();
        for (i, v) in self.values.iter().enumerate() {
            if let Some(val) = v {
                let key = columns
                    .get(i)
                    .cloned()
                    .filter(|c| !c.is_empty())
                    .unwrap_or_else(|| format!("p{i}"));
                map.insert(key, serde_json::Value::String(val.clone()));
            }
        }
        serde_json::Value::Object(map).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_json_string_escapes_special_chars() {
        let params = ParamValues::new(vec![
            Some("val\"with\"quotes".to_string()),
            Some("back\\slash".to_string()),
            Some("new\nline".to_string()),
            None,
        ]);
        let json = params.to_json_string();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["p0"], "val\"with\"quotes");
        assert_eq!(parsed["p1"], "back\\slash");
        assert_eq!(parsed["p2"], "new\nline");
        // None is omitted, not serialized as null
        assert!(parsed.get("p3").is_none());
    }

    #[test]
    fn test_to_json_string_with_columns() {
        let params = ParamValues::new(vec![Some("v1".to_string()), Some("v2".to_string())]);
        let cols = vec!["col_a".to_string(), "col_b".to_string()];
        let json = params.to_json_string_with_columns(&cols);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["col_a"], "v1");
        assert_eq!(parsed["col_b"], "v2");
    }

    #[test]
    fn test_to_json_string_with_columns_fallback_to_positional() {
        let params = ParamValues::new(vec![
            Some("a".to_string()),
            Some("b".to_string()),
            Some("c".to_string()),
        ]);
        let cols = vec!["first".to_string()]; // only 1 col for 3 params
        let json = params.to_json_string_with_columns(&cols);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["first"], "a");
        assert_eq!(parsed["p1"], "b");
        assert_eq!(parsed["p2"], "c");
    }

    #[test]
    fn test_to_json_string_empty() {
        let params = ParamValues::new(vec![]);
        assert_eq!(params.to_json_string(), "{}");
    }
}
