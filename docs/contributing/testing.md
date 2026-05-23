# Testing

## Pyramid

- **Unit tests** — `#[cfg(test)] mod tests` in each file
- **Integration tests** — `tests/*.rs` per crate
- **Property tests** — `proptest` for invariant verification
- **Wire corpus replay** — byte-for-byte PG wire comparison
- **Benchmarks** — `criterion` for performance regression

## Run

```bash
cargo test                                    # All
cargo test -p slateduck-core                  # One crate
cargo bench -p slateduck-catalog              # Benchmarks
```
