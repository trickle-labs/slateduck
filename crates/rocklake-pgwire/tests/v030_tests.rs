//! v0.30.0 — PG-Wire & Protocol Hardening
//!
//! Tests for:
//! 1. Fail-closed binary COPY parser (CopyParseError on truncation/corruption).
//! 2. CLI flags/docs conformance: docs must not advertise flags the binary does not parse.

use rocklake_pgwire::copy_parser::{self, CopyParseError};

// ─── COPY parser signature ────────────────────────────────────────────────────

const PGCOPY_SIGNATURE: &[u8] = b"PGCOPY\n\xff\r\n\0";

/// Build a well-formed binary COPY stream.
fn build_binary_copy(rows: &[&[Option<Vec<u8>>]]) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(PGCOPY_SIGNATURE);
    buf.extend_from_slice(&0i32.to_be_bytes()); // flags
    buf.extend_from_slice(&0i32.to_be_bytes()); // header ext len = 0
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
    buf.extend_from_slice(&(-1i16).to_be_bytes()); // end-of-data marker
    buf
}

// ── 1. Success paths ──────────────────────────────────────────────────────────

#[test]
fn copy_parser_accepts_valid_stream() {
    let val = 99i64.to_be_bytes().to_vec();
    let row: &[Option<Vec<u8>>] = &[Some(val)];
    let data = build_binary_copy(&[row]);
    let rows = copy_parser::parse_binary_copy_rows(&data).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(copy_parser::extract_i64(&rows[0], 0), Some(99));
}

#[test]
fn copy_parser_accepts_empty_stream() {
    let data = build_binary_copy(&[]);
    let rows = copy_parser::parse_binary_copy_rows(&data).unwrap();
    assert!(rows.is_empty());
}

#[test]
fn copy_parser_accepts_null_fields() {
    let row: &[Option<Vec<u8>>] = &[None, Some(b"hello".to_vec())];
    let data = build_binary_copy(&[row]);
    let rows = copy_parser::parse_binary_copy_rows(&data).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0][0], None);
    assert_eq!(
        copy_parser::extract_varchar(&rows[0], 1),
        Some("hello".to_string())
    );
}

// ── 2. Fail-closed: bad signature ─────────────────────────────────────────────

#[test]
fn copy_parser_rejects_empty_input() {
    let err = copy_parser::parse_binary_copy_rows(b"").unwrap_err();
    assert_eq!(err, CopyParseError::TooShort);
}

#[test]
fn copy_parser_rejects_bad_signature() {
    let mut data = vec![0u8; PGCOPY_SIGNATURE.len() + 8];
    data[0] = b'X'; // corrupt first byte
    let err = copy_parser::parse_binary_copy_rows(&data).unwrap_err();
    assert_eq!(err, CopyParseError::BadSignature);
}

// ── 3. Fail-closed: truncation after signature ────────────────────────────────

#[test]
fn copy_parser_rejects_truncation_after_signature() {
    let mut data = Vec::new();
    data.extend_from_slice(PGCOPY_SIGNATURE);
    data.extend_from_slice(&0i32.to_be_bytes()); // flags only — no header ext
    let err = copy_parser::parse_binary_copy_rows(&data).unwrap_err();
    assert!(
        matches!(
            err,
            CopyParseError::TooShort | CopyParseError::TruncatedHeaderExt
        ),
        "expected TooShort or TruncatedHeaderExt, got: {err}"
    );
}

// ── 4. Fail-closed: missing field count ───────────────────────────────────────

#[test]
fn copy_parser_rejects_truncated_field_count() {
    let mut data = Vec::new();
    data.extend_from_slice(PGCOPY_SIGNATURE);
    data.extend_from_slice(&0i32.to_be_bytes()); // flags
    data.extend_from_slice(&0i32.to_be_bytes()); // header ext len = 0
    data.push(0x00); // only 1 byte for the int16 field count
    let err = copy_parser::parse_binary_copy_rows(&data).unwrap_err();
    assert_eq!(err, CopyParseError::TruncatedFieldCount { row: 0 });
}

// ── 5. Fail-closed: mid-field length ──────────────────────────────────────────

#[test]
fn copy_parser_rejects_truncated_field_length() {
    let mut data = Vec::new();
    data.extend_from_slice(PGCOPY_SIGNATURE);
    data.extend_from_slice(&0i32.to_be_bytes()); // flags
    data.extend_from_slice(&0i32.to_be_bytes()); // header ext len = 0
    data.extend_from_slice(&1i16.to_be_bytes()); // field_count = 1
    data.push(0x00); // only 1 of 4 bytes for field length
    let err = copy_parser::parse_binary_copy_rows(&data).unwrap_err();
    assert_eq!(
        err,
        CopyParseError::TruncatedFieldLength { row: 0, field: 0 }
    );
}

// ── 6. Fail-closed: mid-field body ────────────────────────────────────────────

#[test]
fn copy_parser_rejects_truncated_field_body() {
    let mut data = Vec::new();
    data.extend_from_slice(PGCOPY_SIGNATURE);
    data.extend_from_slice(&0i32.to_be_bytes()); // flags
    data.extend_from_slice(&0i32.to_be_bytes()); // header ext len = 0
    data.extend_from_slice(&1i16.to_be_bytes()); // field_count = 1
    data.extend_from_slice(&8i32.to_be_bytes()); // field length = 8
    data.extend_from_slice(&[0u8; 4]); // only 4 bytes instead of 8
    let err = copy_parser::parse_binary_copy_rows(&data).unwrap_err();
    assert_eq!(
        err,
        CopyParseError::TruncatedFieldBody {
            row: 0,
            field: 0,
            declared: 8,
            available: 4,
        }
    );
}

// ── 7. Fail-closed: missing end-of-data marker ────────────────────────────────

#[test]
fn copy_parser_rejects_missing_end_of_data_marker() {
    // One valid row, then stream ends without the -1 trailer.
    let mut data = Vec::new();
    data.extend_from_slice(PGCOPY_SIGNATURE);
    data.extend_from_slice(&0i32.to_be_bytes()); // flags
    data.extend_from_slice(&0i32.to_be_bytes()); // header ext len = 0
                                                 // Row: 1 NULL field.
    data.extend_from_slice(&1i16.to_be_bytes()); // field_count = 1
    data.extend_from_slice(&(-1i32).to_be_bytes()); // NULL field
                                                    // No end-of-data marker — stream ends here.
    let err = copy_parser::parse_binary_copy_rows(&data).unwrap_err();
    // The parser reads the next field_count but hits EOF.
    assert_eq!(err, CopyParseError::TruncatedFieldCount { row: 1 });
}
// ── 8. CLI docs conformance ───────────────────────────────────────────────────

/// The CLI reference must not document flags that the binary does not implement.
///
/// Specifically, `export` should list only `--output` and `--snapshot-id`,
/// and `import` should list only `--input`.
/// The legacy stub flags (`--at-snapshot`, `--at-time`, `--schema`, `--table`,
/// `--merge`, `--dry-run`) must not appear in the relevant sections.
#[test]
fn cli_docs_export_flags_match_implementation() {
    let docs_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/operations/cli-reference.md"
    );
    let content = std::fs::read_to_string(docs_path)
        .expect("cli-reference.md must be readable from workspace root");

    // Extract the `export` section (from the export heading to the next `---`).
    let export_section = extract_section(&content, "### `export`");
    assert!(
        !export_section.contains("--at-snapshot"),
        "cli-reference.md export section must not advertise --at-snapshot (not implemented)"
    );
    assert!(
        !export_section.contains("--at-time"),
        "cli-reference.md export section must not advertise --at-time (not implemented)"
    );
    assert!(
        !export_section.contains("--schema"),
        "cli-reference.md export section must not advertise --schema (not implemented)"
    );
    assert!(
        !export_section.contains("--table"),
        "cli-reference.md export section must not advertise --table (not implemented)"
    );

    // The implemented flags must be present.
    assert!(
        export_section.contains("--output"),
        "cli-reference.md export section must document --output"
    );
    assert!(
        export_section.contains("--snapshot-id"),
        "cli-reference.md export section must document --snapshot-id"
    );
}

#[test]
fn cli_docs_import_flags_match_implementation() {
    let docs_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/operations/cli-reference.md"
    );
    let content = std::fs::read_to_string(docs_path)
        .expect("cli-reference.md must be readable from workspace root");

    let import_section = extract_section(&content, "### `import`");
    assert!(
        !import_section.contains("--merge"),
        "cli-reference.md import section must not advertise --merge (not implemented)"
    );
    assert!(
        !import_section.contains("--dry-run"),
        "cli-reference.md import section must not advertise --dry-run (not implemented)"
    );

    assert!(
        import_section.contains("--input"),
        "cli-reference.md import section must document --input"
    );
}

/// Extract the markdown section starting at `heading` up to the next `---` separator.
fn extract_section(content: &str, heading: &str) -> String {
    let start = content.find(heading).unwrap_or(0);
    let rest = &content[start..];
    // Sections are separated by `\n---\n`.
    let end = rest.find("\n---\n").unwrap_or(rest.len());
    rest[..end].to_owned()
}
