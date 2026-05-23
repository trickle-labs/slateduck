# Development Setup

## Prerequisites

- Rust stable (1.75+)
- Python 3.10+ (for docs)
- DuckDB 1.2.2+ (for integration tests)

## Build

```bash
git clone https://github.com/geir-gronmo/slateduck.git
cd slateduck
cargo build
```

## Test

```bash
cargo test
cargo test -p slateduck-core
cargo test -p slateduck-catalog
```

## Docs

```bash
pip install -r requirements-docs.txt
mkdocs serve
```
