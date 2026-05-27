# Contributing to Rocklake

Thank you for your interest in contributing to Rocklake. Whether you are fixing a typo in documentation, adding a test case, implementing a new feature, or reporting a bug — your contribution makes the project better for everyone who uses it. This section provides everything you need to go from "I want to help" to "my PR is merged" with confidence.

Contributing to an unfamiliar codebase can feel intimidating. The project uses Rust (which has a learning curve), deals with database internals (which are complex), and targets a specific protocol (DuckLake) that you may not have encountered before. This guide is designed to reduce that intimidation. We assume nothing about your prior experience with Rocklake — only that you know enough Rust to compile and run code, and that you are willing to learn the domain-specific details.

The project is structured to make contributions tractable. Each crate has a focused responsibility, the test suite is comprehensive (so you can validate your changes quickly), and the code style is consistent (so you spend less time on formatting decisions and more time on logic). The maintainers are committed to reviewing PRs promptly and providing constructive feedback.

## How to Contribute

The most common contribution types, in rough order of how often they occur:

| Contribution Type | Where to Start | Complexity |
|-------------------|---------------|-----------|
| Report a bug | GitHub Issues | Low |
| Fix documentation | `docs/` directory | Low |
| Add a wire corpus entry | `tests/fixtures/wire-corpus/` | Low–Medium |
| Fix a bug | The relevant crate | Medium |
| Add a test case | `tests/` in the relevant crate | Medium |
| Implement a new SQL statement | `rocklake-sql` + `rocklake-catalog` | Medium–High |
| Add a new catalog table | `rocklake-core` + `rocklake-catalog` | High |
| Performance optimization | `rocklake-catalog` | High |

For your first contribution, we recommend starting with something in the Low or Low–Medium range — a documentation fix, a new wire corpus entry, or a test case. This familiarizes you with the development workflow (clone, branch, change, test, PR) without requiring deep understanding of the codebase.

## Pages in This Section

<div class="grid cards" markdown>

-   **[Development Setup](development-setup.md)**

    ---

    Getting your local environment ready: installing prerequisites, cloning the repository, building the workspace, running tests, and configuring your editor. Covers macOS, Linux, and Windows.

-   **[Architecture Guide](architecture-guide.md)**

    ---

    How the codebase is organized for contributors. The dependency graph between crates, where to make specific types of changes, and the key design principles that guide implementation decisions.

-   **[Code Style](code-style.md)**

    ---

    Coding conventions, naming rules, error handling patterns, module organization, and dependency policy. Following these ensures your PR passes automated checks and aligns with existing code.

-   **[Testing](testing.md)**

    ---

    The multi-layered testing strategy: unit tests, property-based tests, integration tests, wire corpus tests, and golden tests. How to write effective tests and what to test for different types of changes.

-   **[Release Process](release-process.md)**

    ---

    How releases are versioned, built, tagged, and published. Primarily for maintainers but documented publicly for transparency. Includes the hotfix process and support policy.

</div>

## Quick Start for Impatient Contributors

If you want to get started immediately and read details later:

```bash
# Prerequisites: Rust 1.80+ via rustup, Git, C compiler

# Clone and build
git clone https://github.com/rocklake/rocklake.git
cd rocklake
cargo build

# Run the test suite (verify everything works)
cargo test

# Make your change on a branch
git checkout -b fix/my-improvement

# After making changes:
cargo fmt         # Format code
cargo clippy      # Lint
cargo test        # Verify tests pass

# Commit and push
git add .
git commit -m "fix: description of what you fixed"
git push origin fix/my-improvement

# Open a PR on GitHub
```

## Communication

- **Bug reports and feature requests:** GitHub Issues
- **Questions about the codebase:** GitHub Discussions
- **PR discussions:** Directly on the PR

When opening an issue, include:
- Rocklake version (from `rocklake --version` or `Cargo.toml`)
- Storage backend (S3 Standard, S3 Express, GCS, local)
- Steps to reproduce (for bugs)
- Expected vs. actual behavior

## Code of Conduct

Rocklake follows the [Rust Code of Conduct](https://www.rust-lang.org/policies/code-of-conduct). Be respectful, constructive, and collaborative. Toxic behavior, harassment, and discrimination are not tolerated.

## License

Contributions to Rocklake are licensed under the same terms as the project (see the LICENSE file in the repository root). By submitting a PR, you agree that your contribution is licensed under these terms.
