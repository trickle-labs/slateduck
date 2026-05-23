# Contributing to SlateDuck

Thank you for your interest in contributing to SlateDuck!

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
slateduck/
├── Cargo.toml              # Workspace root
├── crates/
│   ├── slateduck-core/     # Foundational types, SlateDB integration
│   ├── slateduck-catalog/  # DuckLake catalog operations
│   ├── slateduck-sql/      # Bounded SQL dispatcher
│   ├── slateduck-sqlite-vfs/ # SQLite VFS layer (future)
│   ├── slateduck-pgwire/   # PostgreSQL wire protocol sidecar
│   └── slateduck-ffi/      # C/C++ FFI bindings (future)
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
