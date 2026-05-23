# Custom Clients

Any DuckLake-compatible client can connect via the PG-wire sidecar.

## Onboarding Process

1. **Capture wire corpus** against PostgreSQL-backed DuckLake
2. **Classify statements** against dispatcher taxonomy
3. **Implement extensions** for Category-B (trivial) shapes
4. **Add replay tests** to CI
5. **Document** in the compatibility matrix

## Categories

- **A (supported):** Matches existing pattern
- **B (trivial extension):** Within bounded set, add behind feature flag
- **C (outside bounded set):** Returns `SQLSTATE 0A000`, by design
