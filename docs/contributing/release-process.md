# Release Process

This page documents how RockLake releases are versioned, built, tested, tagged, and published. It is primarily intended for maintainers who perform releases, but it is documented publicly for transparency — contributors should understand how their merged code reaches users, and users should understand the quality gates that every release passes through.

RockLake follows a deliberate, predictable release cadence. Every release goes through the same process regardless of its size. A one-line bug fix and a major feature addition both pass through the same CI pipeline, the same testing matrix, the same review requirements. This consistency ensures that users can upgrade with confidence — if a release is tagged, it has been validated.

## Version Numbering

RockLake follows [Semantic Versioning](https://semver.org/) (SemVer):

| Version Component | When Incremented | Example |
|-------------------|-----------------|---------|
| **Major** (X.0.0) | Breaking changes to catalog format or public API | 1.0.0 → 2.0.0 |
| **Minor** (0.X.0) | New features, backward-compatible changes | 0.7.0 → 0.8.0 |
| **Patch** (0.0.X) | Bug fixes only, no format or API changes | 0.7.0 → 0.7.1 |

### Pre-1.0 Stability

During the 0.x series (pre-1.0), the project is in active development:

- Minor versions MAY include breaking changes (with advance notice in the changelog)
- The catalog format is not yet frozen (upgrading may require re-creating catalogs)
- The C FFI ABI may change between minor versions
- The wire protocol behavior may change (new statements added, error formats refined)

Once version 1.0.0 is released, these stability guarantees become binding:

- Major versions for breaking changes only
- Minor versions are always backward-compatible
- Patch versions are always safe to apply
- Catalog format changes include migration support

### Version in Code

The version appears in:

- `Cargo.toml` (workspace manifest): defines the workspace version
- Each crate's `Cargo.toml`: references the workspace version
- The binary's `--version` flag output
- The PostgreSQL wire protocol startup message
- The C FFI ABI version constant
- Docker image tags

All of these are updated together in the release PR.

## Release Cadence

RockLake does not follow a fixed time-based release schedule. Instead, releases happen when:

- A meaningful set of changes has accumulated (features, fixes, improvements)
- A critical bug fix needs to reach users quickly (hotfix release)
- A breaking change needs to be shipped (major version bump)

In practice during active development, minor releases happen every 2–4 weeks. Patch releases happen as needed (sometimes within days of a bug report).

## Release Steps

### 1. Prepare the Release

The release process begins when a maintainer decides the current `main` branch is ready for release.

**Version bump:**

Update the version in the workspace `Cargo.toml`:

```toml
[workspace.package]
version = "0.8.0"
```

All crate `Cargo.toml` files reference this via `workspace = true`, so they update automatically.

**Changelog update:**

Update `CHANGELOG.md` with all changes since the last release. Group changes by category:

```markdown
## [0.8.0] - 2025-01-15

### Added
- Support for ALTER TABLE ADD COLUMN (#123)
- Hot key cache for frequently-accessed metadata (#134)
- S3 Express One Zone integration (#140)

### Fixed
- MVCC visibility incorrect for snapshot 0 (#128)
- Wire protocol crash on oversized query strings (#131)

### Changed
- Minimum DuckDB version is now 1.5.2 (#125)
- Default retention increased from 50 to 100 snapshots (#136)

### Performance
- 3x faster prefix scans through batch prefetching (#139)
- Reduced key encoding allocations by 40% (#141)
```

**Full test suite:**

```bash
# Run all tests (unit, integration, property-based)
cargo test

# Run with additional property-test cases
PROPTEST_CASES=2048 cargo test

# Run clippy (must be warning-free)
cargo clippy --all-targets --all-features -- -D warnings

# Verify documentation builds
mkdocs build --strict
```

**Benchmark comparison:**

```bash
# Run benchmarks and compare to the previous release
cargo bench -p rocklake-catalog -- --save-baseline v0.8.0
# ... checkout previous release tag ...
cargo bench -p rocklake-catalog -- --baseline v0.8.0
```

If any benchmark regresses by more than 10%, investigate before releasing. Performance regressions are bugs.

### 2. Create the Release PR

Open a PR titled `release: v0.8.0` containing:

- Version bump in `Cargo.toml`
- Updated `CHANGELOG.md`
- Updated documentation version references (if any)
- Updated benchmark baseline file (if benchmarks changed significantly)

The PR should contain ONLY version-related changes. No feature code, no bug fixes, no refactoring. This makes the release commit trivially reviewable.

**CI runs the full test matrix:**

| Dimension | Values |
|-----------|--------|
| OS | Linux x86_64, macOS ARM64, Windows x86_64 |
| Rust version | Stable, Beta (nightly for informational only) |
| Features | Default, all-features |
| Tests | Unit, integration, property (2048 cases), wire corpus |

All checks must pass. No exceptions.

**Review requirements:**

- At least one maintainer approval
- All CI checks green
- No unresolved review comments
- Changelog accurately reflects all changes since last release

### 3. Merge and Tag

After the PR is approved and CI passes:

```bash
# Merge the release PR (use merge commit, not squash)
# Then tag the merge commit
git checkout main
git pull
git tag v0.8.0
git push origin v0.8.0
```

The tag push triggers the release CI workflow.

### 4. Release CI Workflow

The tag triggers an automated workflow that:

**Builds binaries for all supported platforms:**

| Platform | Architecture | Format |
|----------|-------------|--------|
| Linux | x86_64 | Static binary (musl) |
| Linux | aarch64 | Static binary (musl) |
| macOS | ARM64 (Apple Silicon) | Universal binary |
| macOS | x86_64 (Intel) | Universal binary |
| Windows | x86_64 | .exe |

**Creates a GitHub Release:**

- Attaches all compiled binaries
- Includes the changelog section for this version
- Marks as "pre-release" for 0.x versions
- Generates SHA256 checksums for all artifacts

**Publishes Docker images:**

```
ghcr.io/rocklake/rocklake:0.8.0
ghcr.io/rocklake/rocklake:0.8
ghcr.io/rocklake/rocklake:latest
```

Multi-architecture images (linux/amd64, linux/arm64) built with Docker buildx.

**Deploys documentation:**

- Builds the MkDocs site from the tagged commit
- Deploys to GitHub Pages
- The live documentation always reflects the latest release

**Publishes to crates.io (when applicable):**

Library crates (`rocklake-core`, `rocklake-catalog`) may be published to crates.io for use as Rust dependencies. This happens selectively — not every release publishes to crates.io.

### 5. Post-Release

After the release CI completes:

- **Announce:** Post in GitHub Discussions and relevant community channels
- **Monitor:** Watch for regression reports in the first 24–48 hours
- **Prepare next cycle:** Bump version in `Cargo.toml` to next dev version (e.g., `0.9.0-dev`)

## Hotfix Process

For critical bugs that cannot wait for the next regular release:

### When to Hotfix

- Security vulnerabilities
- Data corruption bugs
- Crashes that affect all users
- Protocol incompatibilities with supported DuckDB versions

### Hotfix Steps

```bash
# Branch from the release tag
git checkout -b hotfix/v0.8.1 v0.8.0

# Apply the minimal fix
# ... edit code ...

# Add a regression test
# ... add test ...

# Verify
cargo test
cargo clippy --all-targets --all-features -- -D warnings

# Commit
git commit -m "fix: correct critical bug X (backport)"

# Push and create PR against main
git push origin hotfix/v0.8.1
```

The hotfix PR follows the same review and CI requirements as a regular release. After merge:

```bash
git tag v0.8.1
git push origin v0.8.1
```

The release CI produces all artifacts as usual.

### Cherry-Picking to Main

If the hotfix was developed on a branch from the release tag, ensure the fix is also present on `main`:

```bash
git checkout main
git cherry-pick <hotfix-commit-hash>
git push origin main
```

## Supported Versions

**Only the latest minor version receives patches.** If the current release is v0.8.0:

- v0.8.x receives bug fixes (hotfixes)
- v0.7.x is unsupported (no further patches)
- v0.6.x is unsupported

Users are expected to track the latest version. The project does not maintain long-term support (LTS) branches.

### Upgrade Path

- Patch upgrades (0.8.0 → 0.8.1): Always safe, no action needed beyond replacing the binary
- Minor upgrades (0.7.x → 0.8.0): Read the changelog for breaking changes (pre-1.0). May require catalog re-creation.
- Major upgrades (1.x → 2.x): Read the migration guide (published with the release)

## Release Artifacts

Each release produces:

| Artifact | Location | Purpose |
|----------|----------|---------|
| Binary (Linux x86_64) | GitHub Release | Direct deployment |
| Binary (Linux ARM64) | GitHub Release | ARM servers, Graviton |
| Binary (macOS ARM64) | GitHub Release | Local development |
| Binary (Windows x64) | GitHub Release | Windows deployment |
| Docker image | ghcr.io | Container deployment |
| Source tarball | GitHub Release | Building from source |
| SHA256 checksums | GitHub Release | Integrity verification |
| Documentation | GitHub Pages | Reference |
| Changelog | Repository + Release notes | What changed |

## Rollback

If a release introduces a critical issue:

1. **Immediate:** Advise users to pin to the previous version
2. **Short-term:** Issue a hotfix release
3. **If needed:** Yank the problematic release from container registries

Binaries on GitHub Releases are never deleted (they are immutable artifacts), but the release can be marked as "known bad" with a prominent warning.

## Further Reading

- **[Development Setup](development-setup.md)** — Building the project
- **[Testing](testing.md)** — The test suite that gates releases
- **[Operations: Upgrades](../operations/upgrades.md)** — How users perform upgrades

---

## v0.9.4 Acceptance Criteria

The following checklist defines the quality bar for the v0.9.4 "GA Ready" milestone.
Every item must be green before the release tag is applied.

### Functional Completeness

- [x] F-11: Concurrent reads — lock is dropped before any async await
- [x] F-13: O(1) `describe_table` — TAG_TABLE_BY_ID secondary index
- [x] F-14: AsyncBridge for DataFusion integration
- [x] F-15: DataFusion Parquet scan with real data (Listing Table API)
- [x] F-20: Writer session regression tests (round-trip create/insert/select)
- [x] F-21: TLS + authentication security protocol tests
- [x] F-22: FFI null-safety tests + DataFusion concurrent read tests
- [x] F-23: `sqlite-vfs` experimental feature gate
- [x] F-24: `MissingParam` structured error + `require_param_u64` helper
- [x] F-25: `#[tracing::instrument]` on 5 key code paths
- [x] Virtual catalog SQL tables (`SELECT * FROM rocklake_catalog.*`)
- [x] DataFusion pg-wire mode (`--datafusion-pg-wire <port>`)
- [x] Spark 3.5 / Trino 432 wire corpus fixtures + classifier tests

### CI Quality Gates

- [x] F-26: CLI smoke test — `rocklake serve --help` validates all documented flags
- [x] F-28: `deny.toml` with ignored transitive advisories
- [x] F-29: MSRV = 1.86 declared in workspace `Cargo.toml`; MSRV verified by security job
- [x] F-33: Coverage job (warns if below 80%) + security job (`cargo deny` + `cargo audit`)
- [x] F-34: `release.yml` GitHub Actions workflow for tagged releases

### Observability

- [x] Zone-map profiling documentation
- [x] `#[tracing::instrument]` on snapshot, GC, excise, repair code paths

### Documentation

- [x] `docs/compatibility.md` — version matrices for all supported integrations
- [x] `docs/contributing/release-process.md` — acceptance criteria (this file)
- [x] `ROADMAP.md` — v0.9.4 section fully checked off

### Test Health

- [x] All workspace tests pass (`cargo test --workspace`)
- [x] `RUSTFLAGS=-Dwarnings cargo clippy --all-targets --all-features` — zero warnings
- [x] `cargo fmt --all -- --check` passes
- [x] `cargo audit` — 0 errors (3 pre-existing warnings are documented and ignored)

