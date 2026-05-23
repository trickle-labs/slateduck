# Release Process

## Steps

1. Update `version` in all `Cargo.toml` files
2. Update ROADMAP.md (check off deliverables)
3. Run: `cargo test && cargo clippy -- -D warnings && cargo fmt --check`
4. Build docs: `mkdocs build --strict`
5. Commit: `git commit -m "release: v0.X.0"`
6. Tag: `git tag v0.X.0`
7. Push: `git push origin main --tags`
8. Create GitHub Release

## CI Gates

- `cargo test`
- `cargo clippy -- -D warnings`
- `cargo fmt --check`
- `mkdocs build --strict`
- Wire corpus replay
