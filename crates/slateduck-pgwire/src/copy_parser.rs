//! Binary PostgreSQL COPY format parser for COPY FROM STDIN streams.
//!
//! DuckDB's ducklake extension initialises the catalog by issuing binary COPY
//! FROM STDIN for several `ducklake_*` tables during the ATTACH sequence.  This
//! module parses those streams so we can bootstrap the catalog store with the
//! exact values DuckDB provides.
//!
//! # Format
//!
//! ```text
//! Signature  : "PGCOPY\n\xff\r\n\0"  (11 bytes)
//! Flags       : int32 (0)
//! Header ext  : int32 length + N bytes
//! For each row:
//!   field_count : int16  (or -1 = end-of-copy trailer)
//!   For each field:
//!     len       : int32  (-1 = NULL)
//!     data      : N bytes (if len >= 0)
//! ```

/// PostgreSQL binary COPY format signature.
const PGCOPY_SIGNATURE: &[u8] = b"PGCOPY\n\xff\r\n\0";

/// Parse a PostgreSQL binary COPY stream.
///
/// Returns the rows as a `Vec<Vec<Option<Vec<u8>>>>` where each inner vec
/// represents a row and each `Option<Vec<u8>>` is either `None` (SQL NULL)
/// or `Some(bytes)` (raw field value in PostgreSQL binary encoding).
///
/// Silently returns whatever rows were decoded before any parse error.
pub fn parse_binary_copy_rows(data: &[u8]) -> Vec<Vec<Option<Vec<u8>>>> {
    let mut pos = 0usize;

    // Validate and skip signature.
    if data.len() < PGCOPY_SIGNATURE.len() + 8 {
        return vec![];
    }
    if !data.starts_with(PGCOPY_SIGNATURE) {
        return vec![];
    }
    pos += PGCOPY_SIGNATURE.len();

    // Skip flags (int32).
    pos += 4;

    // Skip header extension area (int32 length + that many bytes).
    if pos + 4 > data.len() {
        return vec![];
    }
    let ext_len = u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);
    pos += 4;
    pos += ext_len as usize;

    let mut rows: Vec<Vec<Option<Vec<u8>>>> = Vec::new();

    loop {
        // Read field count (int16); -1 signals end-of-copy.
        if pos + 2 > data.len() {
            break;
        }
        let field_count = i16::from_be_bytes([data[pos], data[pos + 1]]);
        pos += 2;

        if field_count < 0 {
            break; // End-of-copy trailer.
        }

        let field_count = field_count as usize;
        let mut row: Vec<Option<Vec<u8>>> = Vec::with_capacity(field_count);

        for _ in 0..field_count {
            if pos + 4 > data.len() {
                return rows; // Truncated data – return what we have.
            }
            let len = i32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);
            pos += 4;

            if len < 0 {
                row.push(None); // SQL NULL.
            } else {
                let len = len as usize;
                if pos + len > data.len() {
                    return rows; // Truncated data.
                }
                row.push(Some(data[pos..pos + len].to_vec()));
                pos += len;
            }
        }

        rows.push(row);
    }

    rows
}

// ─── Field extraction helpers ─────────────────────────────────────────────────

/// Extract a big-endian `i64` from field at `idx` (PostgreSQL `BIGINT` binary).
pub fn extract_i64(row: &[Option<Vec<u8>>], idx: usize) -> Option<i64> {
    let bytes = row.get(idx)?.as_ref()?;
    if bytes.len() == 8 {
        Some(i64::from_be_bytes(
            bytes.as_slice().try_into().expect("length checked"),
        ))
    } else {
        None
    }
}

/// Extract a UTF-8 string from field at `idx` (PostgreSQL `VARCHAR`/`TEXT` binary).
pub fn extract_varchar(row: &[Option<Vec<u8>>], idx: usize) -> Option<String> {
    let bytes = row.get(idx)?.as_ref()?;
    Some(String::from_utf8_lossy(bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_binary_copy(rows: &[&[Option<Vec<u8>>]]) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(PGCOPY_SIGNATURE);
        buf.extend_from_slice(&0i32.to_be_bytes()); // flags
        buf.extend_from_slice(&0i32.to_be_bytes()); // header ext len
        for row in rows {
            buf.extend_from_slice(&(row.len() as i16).to_be_bytes());
            for field in *row {
                match field {
                    None => buf.extend_from_slice(&(-1i32).to_be_bytes()),
                    Some(v) => {
                        buf.extend_from_slice(&(v.len() as i32).to_be_bytes());
                        buf.extend_from_slice(v);
                    }
                }
            }
        }
        buf.extend_from_slice(&(-1i16).to_be_bytes()); // EOF trailer
        buf
    }

    #[test]
    fn parse_empty_copy() {
        let data = build_binary_copy(&[]);
        let rows = parse_binary_copy_rows(&data);
        assert_eq!(rows.len(), 0);
    }

    #[test]
    fn parse_single_int8_row() {
        let val: i64 = 42;
        let bytes = val.to_be_bytes().to_vec();
        let row: &[Option<Vec<u8>>] = &[Some(bytes)];
        let data = build_binary_copy(&[row]);
        let rows = parse_binary_copy_rows(&data);
        assert_eq!(rows.len(), 1);
        assert_eq!(extract_i64(&rows[0], 0), Some(42));
    }

    #[test]
    fn parse_null_field() {
        let row: &[Option<Vec<u8>>] = &[None, Some(b"hello".to_vec())];
        let data = build_binary_copy(&[row]);
        let rows = parse_binary_copy_rows(&data);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][0], None);
        assert_eq!(extract_varchar(&rows[0], 1), Some("hello".to_string()));
    }

    #[test]
    fn parse_schema_name_at_field_4() {
        // ducklake_schema: schema_id, schema_uuid, begin_snapshot, end_snapshot, schema_name, path, path_is_relative
        let schema_id = 1i64.to_be_bytes().to_vec();
        let schema_uuid = vec![0u8; 16]; // 16-byte UUID
        let begin_snap = 1i64.to_be_bytes().to_vec();
        let schema_name = b"main".to_vec();
        let row: &[Option<Vec<u8>>] = &[
            Some(schema_id),
            Some(schema_uuid),
            Some(begin_snap),
            None, // end_snapshot NULL
            Some(schema_name),
            None, // path NULL
            None, // path_is_relative NULL
        ];
        let data = build_binary_copy(&[row]);
        let rows = parse_binary_copy_rows(&data);
        assert_eq!(rows.len(), 1);
        assert_eq!(extract_varchar(&rows[0], 4), Some("main".to_string()));
    }
}
