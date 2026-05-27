# Contributing to Rocklake

Thank you for your interest in contributing to Rocklake!

## Development Setup

1. Install Rust (stable toolchain): https://rustup.rs/
2. Clone the repository
3. Run `cargo build` to compile
4. Run `cargo test --all` to verify everything works

## Code Style

- Run `cargo fmt --all` before committing
- Run `cargo clippy --all-targets --all-features` and fix all warnings
- Tests are required for all new functionality

## Project Structure

```
rocklake/
├── Cargo.toml              # Workspace root
├── crates/
│   ├── rocklake-core/     # Foundational types, SlateDB integration
│   ├── rocklake-catalog/  # DuckLake catalog operations
│   ├── rocklake-sql/      # Bounded SQL dispatcher
│   ├── rocklake-sqlite-vfs/ # SQLite VFS layer (future)
│   ├── rocklake-pgwire/   # PostgreSQL wire protocol sidecar
│   └── rocklake-ffi/      # C/C++ FFI bindings (future)
├── docs/                   # Documentation and design artifacts
├── tests/                  # Integration test fixtures
│   ├── fixtures/           # Wire corpus and handshake captures
│   └── golden/             # Golden reference outputs
└── plans/                  # Design documents
```

## Pull Request Process

1. Create a feature branch from `main`
2. Make your changes with appropriate tests
3. Ensure CI passes (`cargo fmt`, `clippy`, `test`)
4. Submit a PR with a clear description

## License

By contributing, you agree that your contributions will be licensed under
the Apache License 2.0.

## Versioning and Release Policy

### Semantic Versioning

Rocklake follows [Semantic Versioning](https://semver.org/). During the 0.x series, minor versions may include breaking changes with advance notice.

| Component | Breaking | Non-breaking |
|-----------|----------|--------------|
| `catalog-format-version` bump | Major version bump required | — |
| `encoding_version` bump | Minor version bump required | — |
| CLI flag removal | Major (or minor pre-1.0 with notice) | Adding new optional flags |
| SQLSTATE code changes | Major (or minor pre-1.0 with notice) | Adding new SQLSTATE codes |
| C FFI ABI | Major (or minor pre-1.0 with notice) | Adding new optional functions |

### Deprecation Policy

Before removing any of the following, a deprecation notice must appear in `CHANGELOG.md` and the binary for at least six months (or one minor version in the 0.x series), naming the target removal version:

- CLI flags
- Metric names
- SQLSTATE codes
- Public Rust API items marked `pub`
- C FFI functions

### Release Verification Checklist

Before tagging any release:

1. `cargo fmt --all -- --check` — passes
2. `cargo clippy --all-targets --all-features -- -Dwarnings` — zero warnings
3. `cargo test --workspace` — all tests pass
4. `cargo audit` — zero errors
5. `mkdocs build --strict` — documentation builds clean
6. All CI jobs green on the release PR branch
7. `CHANGELOG.md` has an entry for this release
8. Every documented CLI flag is present in `rocklake serve --help` output

### wasmtime Version Upgrade Policy

Rocklake embeds `wasmtime` for WASM UDF execution. The following policy applies:

- **Pinned version:** wasmtime is pinned to a specific major version in the workspace
  `Cargo.toml` (currently `wasmtime = "29"`).
- **Upgrade cadence:** wasmtime major version may be bumped **once per Rocklake
  release cycle**. The bump must be a **dedicated maintenance PR** (not bundled with
  feature work).
- **Upgrade PR requirements:**
  1. Update the version in workspace `Cargo.toml`
  2. Update any fuel API callsites that changed between majors
  3. Re-run the full WASM UDF test suite (`Tier 6f`)
  4. Verify sandbox isolation (no new WASI imports leak through)
  5. Run memory limit regression tests
- **EOL policy:** Staying on an EOL wasmtime major for more than one release cycle
  is **disallowed**. WASM sandbox CVEs accumulate; timely upgrades are a security
  requirement.
- **Fuel API stability:** The fuel metering API is the most common breakage point
  between wasmtime majors. Keep fuel-related code isolated in `wasm_udf.rs` to
  minimize upgrade churn.

See [docs/contributing/release-process.md](docs/contributing/release-process.md) for the full release process including tagging and binary publishing.
