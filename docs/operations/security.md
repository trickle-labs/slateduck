# Security Hardening

This guide documents RockLake's security posture, IAM credential isolation,
SQL injection defenses, TLS configuration, authentication, and the excision
audit trail.

## IAM Credential Isolation

RockLake separates the catalog storage prefix from the data file prefix, enabling
precise IAM policy isolation in object storage.

### Prefix Layout

```
bucket/
  catalogs/          ← SlateDB SST files, manifests, WAL entries
    my-catalog/
      sst-001.sst
      MANIFEST-001
      ...
  data/              ← User Parquet/Arrow/Avro data files
    schema_a/
      table_b/
        part-001.parquet
        ...
```

### IAM Policy Design

**Catalog sidecar (`rocklake serve`)** requires:
- `s3:GetObject`, `s3:PutObject`, `s3:DeleteObject` on `catalogs/*`
- No access to `data/*`

**DuckDB data plane** (reads/writes Parquet files) requires:
- `s3:GetObject`, `s3:PutObject` on `data/*`
- No access to `catalogs/*`

This ensures that a compromise of the DuckDB data-plane credentials cannot
read or tamper with the catalog, and a compromise of the catalog sidecar
cannot access user data files.

### Expected Error: SQLSTATE 42501

When the catalog sidecar attempts to access `data/*` (or vice versa), the
expected SQLSTATE is `42501` (insufficient_privilege). The PG-wire layer maps
`object_store::Error::NotAuthorized` to this code.

### MinIO IAM Setup

```bash
# Create catalog-only policy.
mc admin policy create myminio catalog-policy - <<'EOF'
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Effect": "Allow",
      "Action": ["s3:GetObject", "s3:PutObject", "s3:DeleteObject", "s3:ListBucket"],
      "Resource": ["arn:aws:s3:::mybucket/catalogs/*", "arn:aws:s3:::mybucket"]
    }
  ]
}
EOF

# Create data-only policy.
mc admin policy create myminio data-policy - <<'EOF'
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Effect": "Allow",
      "Action": ["s3:GetObject", "s3:PutObject", "s3:DeleteObject", "s3:ListBucket"],
      "Resource": ["arn:aws:s3:::mybucket/data/*", "arn:aws:s3:::mybucket"]
    }
  ]
}
EOF

# Attach to service accounts.
mc admin user add myminio rocklake-sidecar <password>
mc admin policy attach myminio catalog-policy --user rocklake-sidecar

mc admin user add myminio duckdb-plane <password>
mc admin policy attach myminio data-policy --user duckdb-plane
```

## SQL Injection Defenses

The PG-wire SQL classifier defends against injection at multiple layers:

### Input Validation

| Guard | Description | SQLSTATE |
|-------|-------------|----------|
| NUL byte rejection | Queries containing `\x00` are rejected immediately | `42601` |
| Overlong query rejection | Queries > 1 MiB are rejected | `42000` |
| Non-ASCII keyword bypass | Unicode lookalikes do not match SQL keywords | N/A |
| Parameterized queries | All user-supplied values are bound as parameters, never interpolated | N/A |

### Fuzz Test Results

The `sql_injection_fuzz_zero_panics_zero_wrong_results` test in
`crates/rocklake-pgwire/tests/v040_security_tests.rs` verifies:
- **Zero panics** across 40+ adversarial inputs (NUL bytes, overlong strings,
  nested quotes, Unicode lookalikes, SQL keywords in various cases)
- **Zero wrong results** — every input returns a valid classified outcome or
  a well-defined error

### SQLSTATE Mapping

| Condition | SQLSTATE | Description |
|-----------|----------|-------------|
| NUL byte in query | `42601` | `syntax_error` |
| Query exceeds 1 MiB | `42000` | `syntax_error_or_access_rule_violation` |
| Write to read-only catalog table | `25006` | `read_only_sql_transaction` |
| IAM permission denied | `42501` | `insufficient_privilege` |
| Writer epoch fencing | `40001` | `serialization_failure` |

## TLS Configuration

### Minimum Protocol Version

RockLake enforces a minimum TLS version of **TLS 1.2**. TLS 1.0, TLS 1.1,
and SSL 3.0 are rejected at the server level.

The TLS configuration in `crates/rocklake-pgwire/src/server.rs` uses
`rustls::ServerConfig` with `rustls::server::WebPkiClientVerifier` and
defaults to the `rustls::crypto::ring` provider, which enforces TLS 1.2+.

### CLI Flags

| Flag | Description |
|------|-------------|
| `--tls-cert <path>` | Path to PEM-encoded TLS certificate |
| `--tls-key <path>` | Path to PEM-encoded TLS private key |
| `--tls-required` | Reject plaintext connections |
| `--insecure-no-tls-warning-suppress` | Suppress auth-without-TLS warning |

### Auth Without TLS Warning

When `--auth-user` / `--auth-password` are configured without `--tls-cert` /
`--tls-key`, the server emits this warning at startup:

```
WARN Password authentication is enabled without TLS.
     Credentials will be sent in plaintext.
     Use --tls-cert / --tls-key to enable TLS, or pass
     --insecure-no-tls-warning-suppress if this is intentional.
```

### TLS Version Gating

The `tls_audit_tls_11_and_older_rejected` test in
`crates/rocklake-pgwire/tests/v040_security_tests.rs` verifies that:
- TLS 1.0 (protocol version 0x0301): **rejected**
- TLS 1.1 (protocol version 0x0302): **rejected**
- TLS 1.2 (protocol version 0x0303): **accepted**
- TLS 1.3 (protocol version 0x0304): **accepted**

### `--require-tls` Error Code

When `--tls-required` is set and a plaintext client connects, the server
returns SQLSTATE `28000` (invalid_authorization_specification) in the PG
ErrorResponse message.

## Authentication Security

### Constant-Time Password Comparison

RockLake uses constant-time byte comparison for password verification to
prevent timing side-channel attacks. The implementation:

1. **Never short-circuits** on length mismatch — the full comparison is always
   performed, even when the lengths differ.
2. **XOR-folds** all byte comparisons into a single accumulator, returning
   `true` only if all bytes match.
3. **Does not reveal** whether the user exists or the password length is correct.

The `auth_timing_constant_time_comparison_no_early_exit` and
`auth_timing_wrong_length_no_fast_path_exit` tests in
`crates/rocklake-pgwire/tests/v040_security_tests.rs` verify this contract.

### SCRAM-SHA-256

For production deployments, use `--auth-method scram-sha-256` (the default
when TLS is enabled) instead of plaintext password comparison. SCRAM-SHA-256
provides:
- Mutual authentication (server also proves identity to client)
- Salted, iterated hashing (prevents offline dictionary attacks)
- Channel binding (prevents MITM when combined with TLS)

## Excision Audit Trail

Every `rocklake excise --apply` invocation writes an audit record under the
`0xFF | "excised"` key prefix. This record is:

- **Immutable** — written once, never overwritten
- **Accumulating** — each excision adds a new record
- **Visible to `rocklake diagnose`** — the diagnose report includes a count
  of excision events

### Audit Record Structure

```
Key:   0xFF | "excised" | <timestamp_millis_big_endian>
Value: ExciseAuditEntry {
    timestamp_millis: u64,
    before_snapshot:  u64,
    keys_deleted:     u64,
    keys_failed:      u64,
    operator:         String,
}
```

### Viewing Excision History

```bash
# Show diagnose report including excision audit events.
rocklake diagnose --catalog s3://bucket/catalogs/my-catalog

# The report includes a line like:
# [P2] 3 excision events recorded (latest: 2026-01-15T10:30:00Z by admin@company.com)
```

### Test Coverage

The `excision_audit_trail_*` tests in
`crates/rocklake-pgwire/tests/v040_security_tests.rs` verify:
- The audit key prefix follows the `0xFF | "excised"` convention
- The audit entry contains all required fields
- The audit entry is visible to `list_audit_entries()`
- `excise_plan` on a fresh catalog returns a safe plan

## Security CI Jobs

The following CI jobs enforce security requirements on every pre-release:

| Job | Description | Test File |
|-----|-------------|-----------|
| `security-tests` | PG-wire security tests (existing) | `security_tests.rs` |
| `security-v040` | v0.40.0 new security tests | `v040_security_tests.rs` |
| `tls-compat` | TLS protocol-version gating | `tls_compat_tests.rs` |
| `fault-injection` | Catalog fault injection (existing) | `fault_injection_tests.rs` |
| `fault-injection-v040` | v0.40.0 new fault injection tests | `v040_fault_injection_tests.rs` |

## Related

- [Failover & Kill-9 Recovery](failover.md)
- [Diagnostics](diagnostics.md)
- [Monitoring](monitoring.md)
- [TLS Protocol-Version Gating](../contributing/index.md)
