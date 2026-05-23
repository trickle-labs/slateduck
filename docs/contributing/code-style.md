# Code Style

## Formatting

```bash
cargo fmt --check
cargo fmt
```

## Linting

```bash
cargo clippy -- -D warnings
```

## Naming

| Construct | Convention |
|-----------|-----------|
| Crates | `slateduck-*` |
| Structs | PascalCase |
| Functions | snake_case |
| Constants | SCREAMING_SNAKE |

## Errors

- Per-crate error enum with `thiserror`
- Never `unwrap()` in library code
- Map errors at crate boundaries
