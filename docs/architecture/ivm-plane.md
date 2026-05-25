# IVM Plane Architecture

This document describes the internal architecture of the v0.11 Incremental View Maintenance (IVM) layer.

## Component Overview

```
┌────────────────────────────────────────────────────────────────────┐
│  slateduck-ivm worker process                                       │
│                                                                     │
│  ┌──────────┐    ┌──────────┐    ┌──────────┐    ┌──────────────┐ │
│  │ IvmWorker│───▶│MatviewIn-│───▶│ IvmCircuit│───▶│  output.rs  │ │
│  │ (worker) │    │putSource │    │ (circuit) │    │ (write back)│ │
│  └────┬─────┘    └──────────┘    └──────────┘    └──────────────┘ │
│       │  claim/release lease via CAS                               │
│       ▼                                                             │
│  ┌──────────┐    ┌──────────┐    ┌──────────┐                      │
│  │ CatalogW │    │ IvmTrace │    │ observ-  │                      │
│  │  riter   │    │ (state)  │    │  ability │                      │
│  └──────────┘    └──────────┘    └──────────┘                      │
└────────────────────────────────────────────────────────────────────┘
          │                              ▲
          │ SlateDB KV (via CatalogStore)│
          ▼                              │
┌────────────────────────────────────────────────────────────────────┐
│  Catalog (slateduck-catalog + slateduck-core)                       │
│                                                                     │
│  TAG_MATVIEW (0x1D)          TAG_MATVIEW_DEP (0x1E)                │
│  TAG_MATVIEW_CHECKPOINT (0x1F)  TAG_MATVIEW_SHARD (0x20)           │
└────────────────────────────────────────────────────────────────────┘
```

## Catalog Schema Extensions (v0.11)

Four new catalog tables are registered in `ALL_TAGS`:

### `slateduck_matview` (TAG `0x1D`, Versioned)

Stores the matview definition. Uses MVCC versioning: when status changes, a new row with the same `matview_id` but a higher `begin_snapshot` is staged. Old rows with `end_snapshot` set are invisible at later snapshots.

Key: `0x1D | matview_id(u64 BE) | begin_snapshot(u64 BE)`

### `slateduck_matview_dep` (TAG `0x1E`, AppendOnly)

One row per `(matview_id, base_table_id)` pair. Enables dependency-checking (prevents dropping a table that has active matview dependents).

Key: `0x1E | matview_id(u64 BE) | base_table_id(u64 BE)`

### `slateduck_matview_checkpoint` (TAG `0x1F`, AppendOnly)

Checkpoint watermarks. `seq` is monotone per `(matview_id, shard_id)`. Readers can use the latest checkpoint to compute lag.

Key: `0x1F | matview_id(u64 BE) | shard_id(u32 BE) | seq(u64 BE)`

### `slateduck_matview_shard` (TAG `0x20`, MutableSingleton)

Per-shard lease state. Updated via CAS (`DbTransaction` + `IsolationLevel::SerializableSnapshot`). The `generation` field prevents ABA races: every successful CAS bumps it.

Key: `0x20 | matview_id(u64 BE) | shard_id(u32 BE)`

## IVM Circuit (DBSP Shim)

The `circuit.rs` module implements a lightweight adaptation of DBSP's Z-difference model. The `dbsp` crate (`0.299.0`) is a workspace dependency; `circuit.rs` acts as a compatibility shim between SlateDuck's append-only CDC stream and DBSP's algebraic streaming model.

**Z-difference (ZDelta)**: a row with an integer weight (+1 = insert, -1 = retract). In v0.11 only inserts (+1) are emitted; retraction support is deferred to v0.12.

**State**: `HashMap<group_key_json, AggState>` where `AggState` holds per-aggregate accumulators.

**Ordering**: For MIN/MAX, a `BTreeMap<f64_bits, count>` provides efficient ordered access without external dependencies.

## Lease Protocol

```
Worker                                     SlateDB
  │                                           │
  │── begin(SerializableSnapshot) ──────────▶│
  │◀─ read shard row ───────────────────────│
  │                                           │
  │  Check: owner_worker == "" OR             │
  │         lease_expires_unix_ms < now       │
  │                                           │
  │── put(shard_key, new_row{generation+1}) ─▶│
  │── commit ───────────────────────────────▶│
  │◀─ Ok / ConflictError ───────────────────│
  │                                           │
  │  On ConflictError: retry loop             │
```

## Worker Tick Loop

1. Discover matviews via `list_matviews()` on the latest snapshot.
2. For each shard: call `claim_matview_shard()`.
3. If `Acquired` or `AlreadyOwned`: read new input rows since `last_input_snapshot`.
4. Push rows as `ZDelta{weight: +1}` through `IvmCircuit::push_batch()`.
5. Call `IvmCircuit::read_output()` to get current aggregate state.
6. Write output rows to the output table via `register_inlined_insert()`.
7. Append a `MatviewCheckpointRow` and call `create_snapshot()`.
8. Emit observability events via `tracing`.

## Sharding

Each matview is divided into `shard_count` shards (default 1 in v0.11). Each shard is maintained independently; workers compete for shards via the CAS lease protocol. The `shard_key_column` in `MatviewRow` determines which rows belong to which shard (empty = single-shard mode).

## See Also

- [Key Layout](key-layout.md)
- [MVCC Implementation](mvcc-implementation.md)
- [Concepts: Incremental Views](../concepts/incremental-views.md)
