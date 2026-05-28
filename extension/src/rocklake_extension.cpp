/**
 * rocklake_extension.cpp — DuckDB extension ABI smoke wrapper for the
 * RockLake C FFI.
 *
 * NOTE: This file is an ABI smoke wrapper only. The ATTACH registration
 * required to make `ATTACH 'ducklake:slatedb:...' AS lake` work from DuckDB
 * is pending v0.36.0. The example below describes the planned interface and
 * is NOT yet functional.
 *
 * Planned usage in DuckDB (v0.36.0+):
 *   INSTALL rocklake;
 *   LOAD rocklake;
 *   ATTACH 'ducklake:slatedb:///path/to/catalog' AS lake;  -- planned v0.36.0
 *
 * This extension implements the DuckDB Catalog interface by delegating to the
 * Rust-based rocklake-ffi library through a stable C ABI.
 */

#include "rocklake.h"
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <string>

// ─── Extension Metadata ────────────────────────────────────────────────────

static const char *EXTENSION_NAME = "rocklake";
static const char *EXTENSION_VERSION = "0.5.0";
static const uint32_t EXPECTED_ABI_VERSION = 5000;

// ─── ABI Version Check ─────────────────────────────────────────────────────

static bool verify_abi() {
    uint32_t abi = rocklake_abi_version();
    if (abi != EXPECTED_ABI_VERSION) {
        fprintf(stderr,
                "rocklake extension: ABI version mismatch. "
                "Expected %u, got %u. Please rebuild the extension.\n",
                EXPECTED_ABI_VERSION, abi);
        return false;
    }
    return true;
}

// ─── Catalog Wrapper ────────────────────────────────────────────────────────

/**
 * RockLakeCatalogWrapper wraps the opaque C handle and provides a C++ interface
 * suitable for integration with DuckDB's Catalog system.
 *
 * In a full DuckDB extension, this class would inherit from duckdb::Catalog
 * and implement all required virtual methods. For the beta release, we provide
 * the foundation that can be plugged into DuckDB's extension loading mechanism.
 */
class RockLakeCatalogWrapper {
public:
    RockLakeCatalogWrapper() : catalog_(nullptr) {}

    ~RockLakeCatalogWrapper() {
        if (catalog_) {
            rocklake_close(catalog_);
            catalog_ = nullptr;
        }
    }

    bool Open(const std::string &uri) {
        rocklake_error_t err = {};
        catalog_ = rocklake_open(uri.c_str(), &err);
        if (!catalog_) {
            if (err.message) {
                last_error_ = std::string(err.message);
                rocklake_error_free(&err);
            } else {
                last_error_ = "unknown error opening catalog";
            }
            return false;
        }
        return true;
    }

    rocklake_snapshot_t GetCurrentSnapshot() {
        rocklake_error_t err = {};
        auto snap = rocklake_get_current_snapshot(catalog_, &err);
        if (err.code != ROCKLAKE_OK) {
            if (err.message) {
                last_error_ = std::string(err.message);
                rocklake_error_free(&err);
            }
        }
        return snap;
    }

    rocklake_schema_list_t ListSchemas(uint64_t snapshot_id) {
        rocklake_error_t err = {};
        auto result = rocklake_list_schemas(catalog_, snapshot_id, &err);
        if (err.code != ROCKLAKE_OK && err.message) {
            last_error_ = std::string(err.message);
            rocklake_error_free(&err);
        }
        return result;
    }

    rocklake_table_list_t ListTables(uint64_t schema_id, uint64_t snapshot_id) {
        rocklake_error_t err = {};
        auto result = rocklake_list_tables(catalog_, schema_id, snapshot_id, &err);
        if (err.code != ROCKLAKE_OK && err.message) {
            last_error_ = std::string(err.message);
            rocklake_error_free(&err);
        }
        return result;
    }

    rocklake_column_list_t DescribeTable(uint64_t table_id, uint64_t snapshot_id) {
        rocklake_error_t err = {};
        auto result = rocklake_describe_table(catalog_, table_id, snapshot_id, &err);
        if (err.code != ROCKLAKE_OK && err.message) {
            last_error_ = std::string(err.message);
            rocklake_error_free(&err);
        }
        return result;
    }

    rocklake_file_list_t ListDataFiles(uint64_t table_id, uint64_t snapshot_id) {
        rocklake_error_t err = {};
        auto result = rocklake_list_data_files(catalog_, table_id, snapshot_id, &err);
        if (err.code != ROCKLAKE_OK && err.message) {
            last_error_ = std::string(err.message);
            rocklake_error_free(&err);
        }
        return result;
    }

    const std::string &LastError() const { return last_error_; }

private:
    rocklake_catalog_t *catalog_;
    std::string last_error_;
};

// ─── Extension Entry Point ──────────────────────────────────────────────────

/**
 * DuckDB extension entry point. Called when the extension is loaded.
 *
 * In a full DuckDB community extension, this would:
 * 1. Verify ABI version
 * 2. Register the 'slatedb' catalog type with DuckDB
 * 3. Register the ATTACH handler for 'ducklake:slatedb:' URIs
 *
 * For the beta release, we export the symbols needed for the extension
 * loading mechanism and verify the ABI.
 */
extern "C" {

#ifdef _WIN32
__declspec(dllexport)
#else
__attribute__((visibility("default")))
#endif
const char *rocklake_extension_name() {
    return EXTENSION_NAME;
}

#ifdef _WIN32
__declspec(dllexport)
#else
__attribute__((visibility("default")))
#endif
const char *rocklake_extension_version() {
    return EXTENSION_VERSION;
}

#ifdef _WIN32
__declspec(dllexport)
#else
__attribute__((visibility("default")))
#endif
bool rocklake_extension_init() {
    if (!verify_abi()) {
        return false;
    }
    // Extension loaded successfully — catalog type registration would go here
    // once DuckDB's extension catalog API is available for community extensions.
    return true;
}

} // extern "C"
