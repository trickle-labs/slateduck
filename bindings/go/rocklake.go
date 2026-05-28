// Package rocklake provides Go bindings for the RockLake catalog C ABI.
//
// Usage:
//
//	cat, err := rocklake.Open("/path/to/catalog")
//	if err != nil {
//	    log.Fatal(err)
//	}
//	defer cat.Close()
//
//	snapID, err := cat.SnapshotID()
//	schemas, err := cat.ListSchemas(snapID)
//
// The Go functions wrap the rocklake_* C functions declared in rocklake.h via
// cgo.  Pre-built static libraries for each platform are distributed as GitHub
// release assets so consumers do not need a Rust toolchain.
package rocklake

/*
#cgo CFLAGS: -I${SRCDIR}/../../crates/rocklake-ffi/include
#cgo LDFLAGS: -L${SRCDIR}/../../target/debug -lrocklake_ffi -lpthread -ldl -lm

#include "rocklake.h"
#include <stdlib.h>
#include <string.h>
*/
import "C"
import (
	"fmt"
	"unsafe"
)

// Schema is a catalog schema returned by ListSchemas.
type Schema struct {
	SchemaID   uint64
	SchemaName string
}

// Table is a catalog table returned by ListTables.
type Table struct {
	TableID   uint64
	SchemaID  uint64
	TableName string
}

// DataFile is a data file returned by ListDataFiles.
type DataFile struct {
	DataFileID    uint64
	TableID       uint64
	Path          string
	FileFormat    string
	RowCount      uint64
	FileSizeBytes uint64
	SnapshotID    uint64
}

// Catalog is an open RockLake catalog handle.
type Catalog struct {
	ptr *C.rocklake_catalog_t
}

// Open opens (or creates) a RockLake catalog at uri.
//
// Supported URIs: local filesystem paths and file:// URIs.
// The returned Catalog must be closed with Close() when no longer needed.
func Open(uri string) (*Catalog, error) {
	cURI := C.CString(uri)
	defer C.free(unsafe.Pointer(cURI))

	var err C.rocklake_error_t
	ptr := C.rocklake_open(cURI, &err)
	defer C.rocklake_error_free(&err)

	if ptr == nil {
		msg := goString(C.rocklake_error_message(&err))
		return nil, fmt.Errorf("rocklake.Open: %s (code %d)", msg, C.rocklake_error_code(&err))
	}
	return &Catalog{ptr: ptr}, nil
}

// Close frees the catalog handle.  Safe to call multiple times.
func (c *Catalog) Close() {
	if c.ptr != nil {
		C.rocklake_close(c.ptr)
		c.ptr = nil
	}
}

// SnapshotID returns the current (latest committed) snapshot ID.
// Returns 0 for a fresh catalog with no committed snapshots.
func (c *Catalog) SnapshotID() (uint64, error) {
	var err C.rocklake_error_t
	snap := C.rocklake_get_current_snapshot(c.ptr, &err)
	defer C.rocklake_error_free(&err)
	if code := C.rocklake_error_code(&err); code != C.ROCKLAKE_OK {
		return 0, fmt.Errorf("SnapshotID: %s (code %d)", goString(C.rocklake_error_message(&err)), code)
	}
	return uint64(snap.snapshot_id), nil
}

// ListSchemas returns all schemas visible at snapshotID.
func (c *Catalog) ListSchemas(snapshotID uint64) ([]Schema, error) {
	var err C.rocklake_error_t
	list := C.rocklake_list_schemas(c.ptr, C.uint64_t(snapshotID), &err)
	defer C.rocklake_error_free(&err)
	defer C.rocklake_schema_list_free(&list)

	if code := C.rocklake_error_code(&err); code != C.ROCKLAKE_OK {
		return nil, fmt.Errorf("ListSchemas: %s (code %d)", goString(C.rocklake_error_message(&err)), code)
	}

	count := int(list.count)
	if count == 0 || list.schemas == nil {
		return nil, nil
	}

	schemas := make([]Schema, count)
	slice := (*[1 << 28]C.rocklake_schema_entry_t)(unsafe.Pointer(list.schemas))[:count:count]
	for i, s := range slice {
		schemas[i] = Schema{
			SchemaID:   uint64(s.schema_id),
			SchemaName: C.GoString(s.schema_name),
		}
	}
	return schemas, nil
}

// ListTables returns all tables in schemaID visible at snapshotID.
func (c *Catalog) ListTables(schemaID, snapshotID uint64) ([]Table, error) {
	var err C.rocklake_error_t
	list := C.rocklake_list_tables(c.ptr, C.uint64_t(schemaID), C.uint64_t(snapshotID), &err)
	defer C.rocklake_error_free(&err)
	defer C.rocklake_table_list_free(&list)

	if code := C.rocklake_error_code(&err); code != C.ROCKLAKE_OK {
		return nil, fmt.Errorf("ListTables: %s (code %d)", goString(C.rocklake_error_message(&err)), code)
	}

	count := int(list.count)
	if count == 0 || list.tables == nil {
		return nil, nil
	}

	tables := make([]Table, count)
	slice := (*[1 << 28]C.rocklake_table_entry_t)(unsafe.Pointer(list.tables))[:count:count]
	for i, t := range slice {
		tables[i] = Table{
			TableID:   uint64(t.table_id),
			SchemaID:  uint64(t.schema_id),
			TableName: C.GoString(t.table_name),
		}
	}
	return tables, nil
}

// ListDataFiles returns all data files for tableID visible at snapshotID.
func (c *Catalog) ListDataFiles(tableID, snapshotID uint64) ([]DataFile, error) {
	var err C.rocklake_error_t
	list := C.rocklake_list_data_files(c.ptr, C.uint64_t(tableID), C.uint64_t(snapshotID), &err)
	defer C.rocklake_error_free(&err)
	defer C.rocklake_file_list_free(&list)

	if code := C.rocklake_error_code(&err); code != C.ROCKLAKE_OK {
		return nil, fmt.Errorf("ListDataFiles: %s (code %d)", goString(C.rocklake_error_message(&err)), code)
	}

	count := int(list.count)
	if count == 0 || list.files == nil {
		return nil, nil
	}

	files := make([]DataFile, count)
	slice := (*[1 << 28]C.rocklake_data_file_t)(unsafe.Pointer(list.files))[:count:count]
	for i, f := range slice {
		files[i] = DataFile{
			DataFileID:    uint64(f.data_file_id),
			TableID:       uint64(f.table_id),
			Path:          C.GoString(f.path),
			FileFormat:    C.GoString(f.file_format),
			RowCount:      uint64(f.row_count),
			FileSizeBytes: uint64(f.file_size_bytes),
			SnapshotID:    uint64(f.snapshot_id),
		}
	}
	return files, nil
}

// ABIVersion returns the compile-time ABI version constant from the library.
func ABIVersion() uint32 {
	return uint32(C.rocklake_abi_version())
}

// goString is a nil-safe wrapper around C.GoString.
func goString(p *C.char) string {
	if p == nil {
		return ""
	}
	return C.GoString(p)
}
