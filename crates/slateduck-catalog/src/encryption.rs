//! Encryption support: configuring SlateDB block transformers for at-rest encryption.
//!
//! Uses SlateDB's block-level encryption for catalog values.
//! Parquet encryption is a separate, Parquet-native concern.

/// Encryption configuration for the catalog store.
#[derive(Debug, Clone)]
pub struct EncryptionConfig {
    /// AES-256 encryption key (32 bytes).
    pub key: [u8; 32],
}

impl EncryptionConfig {
    /// Create encryption config from a hex-encoded key string.
    pub fn from_hex(hex_key: &str) -> Result<Self, EncryptionError> {
        let bytes = hex_decode(hex_key)?;
        if bytes.len() != 32 {
            return Err(EncryptionError::InvalidKeyLength {
                expected: 32,
                actual: bytes.len(),
            });
        }
        let mut key = [0u8; 32];
        key.copy_from_slice(&bytes);
        Ok(Self { key })
    }

    /// Create encryption config from raw bytes.
    pub fn from_bytes(key: [u8; 32]) -> Self {
        Self { key }
    }
}

/// Errors related to encryption configuration.
#[derive(Debug, Clone, thiserror::Error)]
pub enum EncryptionError {
    #[error("invalid key length: expected {expected} bytes, got {actual}")]
    InvalidKeyLength { expected: usize, actual: usize },
    #[error("invalid hex encoding: {0}")]
    InvalidHex(String),
}

/// Decode a hex string to bytes.
fn hex_decode(s: &str) -> Result<Vec<u8>, EncryptionError> {
    if !s.len().is_multiple_of(2) {
        return Err(EncryptionError::InvalidHex(
            "odd length hex string".to_string(),
        ));
    }
    let mut bytes = Vec::with_capacity(s.len() / 2);
    for i in (0..s.len()).step_by(2) {
        let byte = u8::from_str_radix(&s[i..i + 2], 16)
            .map_err(|e| EncryptionError::InvalidHex(e.to_string()))?;
        bytes.push(byte);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_hex_valid() {
        let hex = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let config = EncryptionConfig::from_hex(hex).unwrap();
        assert_eq!(config.key[0], 0x01);
        assert_eq!(config.key[31], 0xef);
    }

    #[test]
    fn test_from_hex_invalid_length() {
        let hex = "0123456789abcdef";
        let err = EncryptionConfig::from_hex(hex).unwrap_err();
        assert!(matches!(err, EncryptionError::InvalidKeyLength { .. }));
    }

    #[test]
    fn test_from_hex_invalid_chars() {
        let hex = "zz23456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let err = EncryptionConfig::from_hex(hex).unwrap_err();
        assert!(matches!(err, EncryptionError::InvalidHex(_)));
    }
}
