/**
 * rocklake.h — C ABI header for the RockLake catalog client library.
 *
 * This header is the stable interface consumed by:
 *   - Language bindings (Python/PyO3, Go/cgo, Node.js/napi-rs)
 *   - The native DuckDB extension (v0.36.0)
 *   - Any other embedding caller
 *
 * ABI Contract: callers MUST call rocklake_abi_version() at load time and
 * refuse to proceed when the version does not match ROCKLAKE_ABI_VERSION.
 *
 * Thread-Safety Contract (applies to all functions in this header):
 *
 *   - A `rocklake_catalog_t *` handle must NOT be shared across threads
 *     unless the caller provides external mutual-exclusion.
 *   - Two calls that use the same catalog handle concurrently produce
 *     undefined behaviour.
 *   - Different catalog handles (from separate `rocklake_open()` calls)
 *     may be used concurrently on different threads.
 *
 * Ownership Contract:
 *
 *   - Functions that return a heap-allocated type are annotated with the
 *     `rocklake_*_free()` function the caller must invoke.
 *   - Every `rocklake_error_t` output parameter must be freed with
 *     `rocklake_error_free()` even when the call succeeds.
 *   - Pointer fields inside returned structs (e.g. `schema_name`) are
 *     owned by the struct and released by the corresponding `_free()`
 *     function; callers must not free them independently.
 *
 * See docs/reference/c-api.md for the full function reference and
 * docs/architecture/ffi-safety.md for the ownership model.
 */

#ifndef ROCKLAKE_H
#define ROCKLAKE_H

#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ─── ABI Version ──────────────────────────────────────────────────────── */

/**
 * Compile-time ABI version constant (major * 1000 + minor).
 *
 * Compare against rocklake_abi_version() at runtime; refuse to load if
 * the values differ.
 */
#define ROCKLAKE_ABI_VERSION 5000U

/**
 * Returns the ABI version (major * 1000 + minor).
 *
 * Thread-safety: safe to call from any thread at any time.
 * Ownership: returns a plain integer — no memory to free.
 */
uint32_t rocklake_abi_version(void);

/* ─── Error Handling ───────────────────────────────────────────────────── */

/**
 * Error value returned as an output parameter from most FFI functions.
 *
 * Ownership: The `message` field is a heap-allocated C string owned by this
 * struct. Call `rocklake_error_free()` to release it — even when
 * `code == ROCKLAKE_OK`.
 *
 * Nullability: Functions that accept `rocklake_error_t *err` treat a NULL
 * `err` as "caller does not want error details"; the error is still
 * reflected in the return value.
 */
typedef struct {
    /** ROCKLAKE_OK (0) on success; a non-zero ROCKLAKE_ERR_* code on failure. */
    int32_t code;
    /** Human-readable message. NULL when code == ROCKLAKE_OK.
     *  Caller-owned; release with rocklake_error_free(). */
    char *message;
} rocklake_error_t;

/** Error codes */
enum {
    ROCKLAKE_OK = 0,
    ROCKLAKE_ERR_INTERNAL = 1,
    ROCKLAKE_ERR_NOT_FOUND = 2,
    ROCKLAKE_ERR_WRITER_FENCED = 3,
    ROCKLAKE_ERR_FORMAT_MISMATCH = 4,
    ROCKLAKE_ERR_VALUE_TOO_LARGE = 5,
    ROCKLAKE_ERR_TRANSACTION_CONFLICT = 6,
    ROCKLAKE_ERR_NOT_INITIALIZED = 7,
    ROCKLAKE_ERR_INVALID_HANDLE = 8,
    /* POSIX-style short aliases */
    ROCKLAKE_ERR_FENCED = 3,    /* alias for ROCKLAKE_ERR_WRITER_FENCED */
    ROCKLAKE_ERR_CONFLICT = 6,  /* alias for ROCKLAKE_ERR_TRANSACTION_CONFLICT */
};

/**
 * Returns the integer error code. Borrowing; no memory transferred.
 * Nullability: `err` may be NULL; returns 0 (ROCKLAKE_OK).
 */
int32_t rocklake_error_code(const rocklake_error_t *err);
/**
 * Returns a borrowed pointer to the error message (do NOT free it directly).
 * Nullability: `err` may be NULL; returns NULL.
 */
const char *rocklake_error_message(const rocklake_error_t *err);
/**
 * Frees the message field and zeroes the struct.
 * Must be called on every rocklake_error_t output, including on success.
 * Nullability: `err` may be NULL; safe no-op.
 */
void rocklake_error_free(rocklake_error_t *err);

/* ─── Opaque Catalog Handle ────────────────────────────────────────────── */

/**
 * Opaque handle representing an open RockLake catalog.
 *
 * Ownership: Created by `rocklake_open()`; must be closed with
 *   `rocklake_close()` exactly once.
 * Thread-safety: A single handle must NOT be used from multiple threads
 *   simultaneously without external locking.
 * Nullability: Functions that accept this type treat NULL as an invalid
 *   handle and return ROCKLAKE_ERR_NOT_INITIALIZED.
 */
typedef struct RockLakeCatalog rocklake_catalog_t;

/* ─── Result Types ─────────────────────────────────────────────────────── */

/**
 * Snapshot metadata returned by value — no heap allocation, no free needed.
 */
typedef struct {
    /** Monotonic snapshot identifier. 0 when no snapshots exist. */
    uint64_t snapshot_id;
    /** Catalog schema version at this snapshot. */
    uint64_t schema_version;
} rocklake_snapshot_t;

typedef struct {
    uint64_t data_file_id;
    uint64_t table_id;
    /** Heap-allocated path string. Released by rocklake_file_list_free(). */
    char *path;
    /** Heap-allocated format string (e.g. "parquet"). Released by rocklake_file_list_free(). */
    char *file_format;
    uint64_t row_count;
    uint64_t file_size_bytes;
    uint64_t snapshot_id;
} rocklake_data_file_t;

/**
 * Caller-owned list of data files. Free with `rocklake_file_list_free()`.
 */
typedef struct {
    rocklake_data_file_t *files;
    uint64_t count;
} rocklake_file_list_t;

typedef struct {
    uint64_t schema_id;
    /** Heap-allocated schema name. Released by rocklake_schema_list_free(). */
    char *schema_name;
} rocklake_schema_entry_t;

/**
 * Caller-owned list of schemas. Free with `rocklake_schema_list_free()`.
 */
typedef struct {
    rocklake_schema_entry_t *schemas;
    uint64_t count;
} rocklake_schema_list_t;

typedef struct {
    uint64_t table_id;
    uint64_t schema_id;
    /** Heap-allocated table name. Released by rocklake_table_list_free(). */
    char *table_name;
} rocklake_table_entry_t;

/**
 * Caller-owned list of tables. Free with `rocklake_table_list_free()`.
 */
typedef struct {
    rocklake_table_entry_t *tables;
    uint64_t count;
} rocklake_table_list_t;

typedef struct {
    uint64_t column_id;
    uint64_t table_id;
    /** Heap-allocated column name. Released by rocklake_column_list_free(). */
    char *column_name;
    /** Heap-allocated SQL type name. Released by rocklake_column_list_free(). */
    char *data_type;
    uint64_t column_index;
    bool is_nullable;
} rocklake_column_entry_t;

/**
 * Caller-owned list of columns. Free with `rocklake_column_list_free()`.
 */
typedef struct {
    rocklake_column_entry_t *columns;
    uint64_t count;
} rocklake_column_list_t;

/* ─── Catalog Operations ───────────────────────────────────────────────── */

/**
 * Open a catalog at the given URI (local filesystem path).
 *
 * Thread-safety: safe to call concurrently; each call returns an independent
 *   handle backed by its own Tokio runtime.
 * Nullability: `uri` must be non-NULL. `err` may be NULL.
 * Ownership: on success returns a heap-allocated handle the caller must
 *   close with `rocklake_close()`. Returns NULL on failure.
 * Free: `rocklake_close()` for the handle; `rocklake_error_free(err)`.
 */
rocklake_catalog_t *rocklake_open(const char *uri, rocklake_error_t *err);

/**
 * Open a catalog in **read-only** mode: no writer epoch is acquired.
 *
 * Identical to `rocklake_open()` except that the CAS writer-epoch key is not
 * incremented.  Many simultaneous `rocklake_open_readonly()` calls against the
 * same catalog prefix produce zero write conflicts — use this function for
 * stateless reader replicas, analytics sidecars, and horizontal fan-out pods.
 *
 * Thread-safety: safe to call concurrently; each call returns an independent
 *   handle backed by its own Tokio runtime.
 * Nullability: `uri` must be non-NULL. `err` may be NULL.
 * Ownership: on success returns a heap-allocated handle the caller must
 *   close with `rocklake_close()`. Returns NULL on failure.
 * Free: `rocklake_close()` for the handle; `rocklake_error_free(err)`.
 */
rocklake_catalog_t *rocklake_open_readonly(const char *uri, rocklake_error_t *err);

/**
 * Close and free a catalog handle.
 *
 * Thread-safety: must not be called while any other thread uses this handle.
 * Nullability: `catalog` may be NULL or already-closed; safe no-op.
 * Ownership: releases all memory. Calling twice on the same pointer is safe.
 */
void rocklake_close(rocklake_catalog_t *catalog);

/**
 * Get the current (latest) snapshot.
 *
 * Nullability: `catalog` must be non-NULL and open. `err` may be NULL.
 * Ownership: returns by value (no heap allocation). Free `err` with
 *   `rocklake_error_free()`.
 */
rocklake_snapshot_t rocklake_get_current_snapshot(
    rocklake_catalog_t *catalog, rocklake_error_t *err);

/**
 * List schemas at a given snapshot.
 *
 * Nullability: `catalog` must be non-NULL and open. `err` may be NULL.
 * Ownership: returns a heap-allocated `rocklake_schema_list_t`.
 * Free: `rocklake_schema_list_free()`, `rocklake_error_free(err)`.
 */
rocklake_schema_list_t rocklake_list_schemas(
    rocklake_catalog_t *catalog, uint64_t snapshot_id, rocklake_error_t *err);

/**
 * List tables in a schema at a given snapshot.
 *
 * Nullability: `catalog` must be non-NULL and open. `err` may be NULL.
 * Ownership: returns a heap-allocated `rocklake_table_list_t`.
 * Free: `rocklake_table_list_free()`, `rocklake_error_free(err)`.
 */
rocklake_table_list_t rocklake_list_tables(
    rocklake_catalog_t *catalog, uint64_t schema_id, uint64_t snapshot_id,
    rocklake_error_t *err);

/**
 * Describe a table's columns at a given snapshot.
 *
 * Nullability: `catalog` must be non-NULL and open. `err` may be NULL.
 * Ownership: returns a heap-allocated `rocklake_column_list_t`.
 * Free: `rocklake_column_list_free()`, `rocklake_error_free(err)`.
 */
rocklake_column_list_t rocklake_describe_table(
    rocklake_catalog_t *catalog, uint64_t table_id, uint64_t snapshot_id,
    rocklake_error_t *err);

/**
 * List data files for a table at a given snapshot.
 *
 * Nullability: `catalog` must be non-NULL and open. `err` may be NULL.
 * Ownership: returns a heap-allocated `rocklake_file_list_t`.
 * Free: `rocklake_file_list_free()`, `rocklake_error_free(err)`.
 */
rocklake_file_list_t rocklake_list_data_files(
    rocklake_catalog_t *catalog, uint64_t table_id, uint64_t snapshot_id,
    rocklake_error_t *err);

/* ─── Free Functions ───────────────────────────────────────────────────── */

/**
 * Free a schema list returned by `rocklake_list_schemas()`.
 * Nullability: `list` may be NULL; safe no-op.
 * Do NOT free individual `schema_name` pointers before calling this.
 */
void rocklake_schema_list_free(rocklake_schema_list_t *list);
/**
 * Free a table list returned by `rocklake_list_tables()`.
 * Nullability: `list` may be NULL.
 */
void rocklake_table_list_free(rocklake_table_list_t *list);
/**
 * Free a column list returned by `rocklake_describe_table()`.
 * Nullability: `list` may be NULL.
 */
void rocklake_column_list_free(rocklake_column_list_t *list);
/**
 * Free a file list returned by `rocklake_list_data_files()`.
 * Nullability: `list` may be NULL.
 */
void rocklake_file_list_free(rocklake_file_list_t *list);

#ifdef __cplusplus
}
#endif

#endif /* ROCKLAKE_H */
