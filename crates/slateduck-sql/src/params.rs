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
}
