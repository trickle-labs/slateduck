# Immutability Trade-offs

## Benefits

- **Read scale-out for free** — readers never coordinate with writer
- **Time travel as natural read mode** — every snapshot is a prefix scan
- **Auditable fact log** — every change preserved
- **Future-proof substrate** — multiple schemas can share storage

## Costs

- **Monotonic storage growth** — every retired version occupies space
- **Compaction overhead** — more data means more compaction work
- **Operator responsibility** — unlike autovacuum, cleanup is explicit

## Design Principle

Creating data is cheap and automatic. Deleting data is expensive and explicit. This asymmetry ensures data loss requires an explicit decision — never an accident.
