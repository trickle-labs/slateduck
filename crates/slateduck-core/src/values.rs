//! Value encoding and decoding for the SlateDuck catalog.
//!
//! Every value is prefixed with: `encoding_version: u8 | magic: b"SDKV" | payload`
//! Old readers encountering an unknown encoding_version return an explicit error.

use prost::Message;

/// Magic bytes that identify a SlateDuck catalog value.
pub const VALUE_MAGIC: &[u8; 4] = b"SDKV";

/// Current encoding version.
pub const ENCODING_VERSION: u8 = 1;

/// Errors during value encoding/decoding.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ValueError {
    #[error("value too short: expected at least {expected} bytes, got {actual}")]
    TooShort { expected: usize, actual: usize },
    #[error("invalid magic bytes: expected SDKV, got {0:?}")]
    InvalidMagic([u8; 4]),
    #[error("unsupported encoding version {0}; this build supports version {ENCODING_VERSION}")]
    UnsupportedVersion(u8),
    #[error("protobuf decode error: {0}")]
    DecodeError(String),
    #[error("value exceeds maximum size of {max} bytes (actual: {actual})")]
    TooLarge { max: usize, actual: usize },
}

/// Maximum encoded value size (64 MiB) for inlined rows.
pub const MAX_INLINED_VALUE_SIZE: usize = 64 * 1024 * 1024;

/// Encode a protobuf message into a SlateDuck value envelope.
pub fn encode_value<M: Message>(msg: &M) -> Vec<u8> {
    let payload = msg.encode_to_vec();
    let mut buf = Vec::with_capacity(1 + 4 + payload.len());
    buf.push(ENCODING_VERSION);
    buf.extend_from_slice(VALUE_MAGIC);
    buf.extend_from_slice(&payload);
    buf
}

/// Decode the raw payload bytes from a SlateDuck value envelope.
/// Returns the payload slice (after magic+version).
pub fn decode_value_envelope(data: &[u8]) -> Result<&[u8], ValueError> {
    if data.len() < 5 {
        return Err(ValueError::TooShort {
            expected: 5,
            actual: data.len(),
        });
    }
    let version = data[0];
    if version != ENCODING_VERSION {
        return Err(ValueError::UnsupportedVersion(version));
    }
    let magic: [u8; 4] = data[1..5]
        .try_into()
        .expect("bounds verified: data.len() >= 5");
    if &magic != VALUE_MAGIC {
        return Err(ValueError::InvalidMagic(magic));
    }
    Ok(&data[5..])
}

/// Decode a protobuf message from a SlateDuck value envelope.
pub fn decode_value<M: Message + Default>(data: &[u8]) -> Result<M, ValueError> {
    let payload = decode_value_envelope(data)?;
    M::decode(payload).map_err(|e| ValueError::DecodeError(e.to_string()))
}

/// Encode a raw u64 counter value (no protobuf, just the envelope + 8 bytes BE).
pub fn encode_counter(val: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(13);
    buf.push(ENCODING_VERSION);
    buf.extend_from_slice(VALUE_MAGIC);
    buf.extend_from_slice(&val.to_be_bytes());
    buf
}

/// Decode a raw u64 counter value.
pub fn decode_counter(data: &[u8]) -> Result<u64, ValueError> {
    let payload = decode_value_envelope(data)?;
    if payload.len() < 8 {
        return Err(ValueError::TooShort {
            expected: 8,
            actual: payload.len(),
        });
    }
    Ok(u64::from_be_bytes(
        payload[..8]
            .try_into()
            .expect("bounds verified by caller: payload.len() >= 8"),
    ))
}

/// Encode a raw u32 value (for catalog-format-version).
pub fn encode_format_version(val: u32) -> Vec<u8> {
    let mut buf = Vec::with_capacity(9);
    buf.push(ENCODING_VERSION);
    buf.extend_from_slice(VALUE_MAGIC);
    buf.extend_from_slice(&val.to_be_bytes());
    buf
}

/// Decode a raw u32 value (for catalog-format-version).
pub fn decode_format_version(data: &[u8]) -> Result<u32, ValueError> {
    let payload = decode_value_envelope(data)?;
    if payload.len() < 4 {
        return Err(ValueError::TooShort {
            expected: 4,
            actual: payload.len(),
        });
    }
    Ok(u32::from_be_bytes(
        payload[..4]
            .try_into()
            .expect("bounds verified by caller: payload.len() >= 4"),
    ))
}

/// Encode raw bytes inside the SlateDuck value envelope.
/// Used for JSON and other non-protobuf payloads (e.g. audit log entries).
pub fn encode_raw_value(data: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + 4 + data.len());
    buf.push(ENCODING_VERSION);
    buf.extend_from_slice(VALUE_MAGIC);
    buf.extend_from_slice(data);
    buf
}

/// Decode raw bytes from a SlateDuck value envelope.
/// Returns the raw payload (after magic+version verification).
pub fn decode_raw_value(data: &[u8]) -> Result<Vec<u8>, ValueError> {
    let payload = decode_value_envelope(data)?;
    Ok(payload.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_round_trip() {
        let encoded = encode_counter(42);
        let decoded = decode_counter(&encoded).unwrap();
        assert_eq!(decoded, 42);
    }

    #[test]
    fn format_version_round_trip() {
        let encoded = encode_format_version(1);
        let decoded = decode_format_version(&encoded).unwrap();
        assert_eq!(decoded, 1);
    }

    #[test]
    fn invalid_magic() {
        let data = vec![ENCODING_VERSION, b'X', b'X', b'X', b'X', 0, 0, 0, 0];
        let err = decode_counter(&data).unwrap_err();
        assert!(matches!(err, ValueError::InvalidMagic(_)));
    }

    #[test]
    fn unsupported_version() {
        let mut data = encode_counter(1);
        data[0] = 99; // Unknown version
        let err = decode_counter(&data).unwrap_err();
        assert!(matches!(err, ValueError::UnsupportedVersion(99)));
    }

    #[test]
    fn too_short() {
        let err = decode_counter(&[1, 2]).unwrap_err();
        assert!(matches!(err, ValueError::TooShort { .. }));
    }
}
