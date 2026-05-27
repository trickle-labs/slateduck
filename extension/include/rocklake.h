/**
 * rocklake.h — C ABI header for the Rocklake catalog FFI layer.
 *
 * This header is consumed by the DuckDB extension to call into the Rust
 * catalog implementation. All types use stable C representations.
 *
 * ABI Contract: The extension MUST check rocklake_abi_version() at load
 * time and refuse to proceed on version mismatch.
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
 * Returns the ABI version (major * 1000 + minor).
 * Extension checks this at load time; mismatch → refuse to load.
 */
uint32_t rocklake_abi_version(void);

/* ─── Error Handling ───────────────────────────────────────────────────── */

typedef struct {
    int32_t code;
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
};

int32_t rocklake_error_code(const rocklake_error_t *err);
const char *rocklake_error_message(const rocklake_error_t *err);
void rocklake_error_free(rocklake_error_t *err);

/* ─── Opaque Catalog Handle ────────────────────────────────────────────── */

typedef struct RocklakeCatalog rocklake_catalog_t;

/* ─── Result Types ─────────────────────────────────────────────────────── */

typedef struct {
    uint64_t snapshot_id;
    uint64_t schema_version;
} rocklake_snapshot_t;

typedef struct {
    uint64_t data_file_id;
    uint64_t table_id;
    char *path;
    char *file_format;
    uint64_t row_count;
    uint64_t file_size_bytes;
    uint64_t snapshot_id;
} rocklake_data_file_t;

typedef struct {
    rocklake_data_file_t *files;
    uint64_t count;
} rocklake_file_list_t;

typedef struct {
    uint64_t schema_id;
    char *schema_name;
} rocklake_schema_entry_t;

typedef struct {
    rocklake_schema_entry_t *schemas;
    uint64_t count;
} rocklake_schema_list_t;

typedef struct {
    uint64_t table_id;
    uint64_t schema_id;
    char *table_name;
} rocklake_table_entry_t;

typedef struct {
    rocklake_table_entry_t *tables;
    uint64_t count;
} rocklake_table_list_t;

typedef struct {
    uint64_t column_id;
    uint64_t table_id;
    char *column_name;
    char *data_type;
    uint64_t column_index;
    bool is_nullable;
} rocklake_column_entry_t;

typedef struct {
    rocklake_column_entry_t *columns;
    uint64_t count;
} rocklake_column_list_t;

/* ─── Catalog Operations ───────────────────────────────────────────────── */

/**
 * Open a catalog at the given URI (local filesystem path).
 * Returns NULL on failure; check err for details.
 */
rocklake_catalog_t *rocklake_open(const char *uri, rocklake_error_t *err);

/** Close and free a catalog handle. */
void rocklake_close(rocklake_catalog_t *catalog);

/** Get the current (latest) snapshot. */
rocklake_snapshot_t rocklake_get_current_snapshot(
    rocklake_catalog_t *catalog, rocklake_error_t *err);

/** List schemas at a given snapshot. */
rocklake_schema_list_t rocklake_list_schemas(
    rocklake_catalog_t *catalog, uint64_t snapshot_id, rocklake_error_t *err);

/** List tables in a schema at a given snapshot. */
rocklake_table_list_t rocklake_list_tables(
    rocklake_catalog_t *catalog, uint64_t schema_id, uint64_t snapshot_id,
    rocklake_error_t *err);

/** Describe a table's columns at a given snapshot. */
rocklake_column_list_t rocklake_describe_table(
    rocklake_catalog_t *catalog, uint64_t table_id, uint64_t snapshot_id,
    rocklake_error_t *err);

/** List data files for a table at a given snapshot. */
rocklake_file_list_t rocklake_list_data_files(
    rocklake_catalog_t *catalog, uint64_t table_id, uint64_t snapshot_id,
    rocklake_error_t *err);

/* ─── Free Functions ───────────────────────────────────────────────────── */

void rocklake_schema_list_free(rocklake_schema_list_t *list);
void rocklake_table_list_free(rocklake_table_list_t *list);
void rocklake_column_list_free(rocklake_column_list_t *list);
void rocklake_file_list_free(rocklake_file_list_t *list);

#ifdef __cplusplus
}
#endif

#endif /* ROCKLAKE_H */
