package rocklake_test

import (
	"os"
	"testing"

	rocklake "github.com/trickle-labs/rocklake-go"
)

func tempDir(t *testing.T) string {
	t.Helper()
	dir, err := os.MkdirTemp("", "rocklake-go-test-*")
	if err != nil {
		t.Fatalf("tempdir: %v", err)
	}
	t.Cleanup(func() { os.RemoveAll(dir) })
	return dir
}

func TestABIVersion(t *testing.T) {
	v := rocklake.ABIVersion()
	if v == 0 {
		t.Errorf("ABIVersion must be > 0, got %d", v)
	}
}

func TestOpenClose(t *testing.T) {
	dir := tempDir(t)
	cat, err := rocklake.Open(dir)
	if err != nil {
		t.Fatalf("Open: %v", err)
	}
	cat.Close()
}

func TestSnapshotIDFreshCatalog(t *testing.T) {
	dir := tempDir(t)
	cat, err := rocklake.Open(dir)
	if err != nil {
		t.Fatalf("Open: %v", err)
	}
	defer cat.Close()

	snap, err := cat.SnapshotID()
	if err != nil {
		t.Fatalf("SnapshotID: %v", err)
	}
	if snap != 0 {
		t.Errorf("expected snapshot_id=0 for fresh catalog, got %d", snap)
	}
}

func TestListSchemasEmpty(t *testing.T) {
	dir := tempDir(t)
	cat, err := rocklake.Open(dir)
	if err != nil {
		t.Fatalf("Open: %v", err)
	}
	defer cat.Close()

	schemas, err := cat.ListSchemas(0)
	if err != nil {
		t.Fatalf("ListSchemas: %v", err)
	}
	if len(schemas) != 0 {
		t.Errorf("expected 0 schemas, got %d", len(schemas))
	}
}

func TestListTablesEmpty(t *testing.T) {
	dir := tempDir(t)
	cat, err := rocklake.Open(dir)
	if err != nil {
		t.Fatalf("Open: %v", err)
	}
	defer cat.Close()

	tables, err := cat.ListTables(1, 0)
	if err != nil {
		t.Fatalf("ListTables: %v", err)
	}
	if len(tables) != 0 {
		t.Errorf("expected 0 tables, got %d", len(tables))
	}
}

func TestListDataFilesEmpty(t *testing.T) {
	dir := tempDir(t)
	cat, err := rocklake.Open(dir)
	if err != nil {
		t.Fatalf("Open: %v", err)
	}
	defer cat.Close()

	files, err := cat.ListDataFiles(1, 0)
	if err != nil {
		t.Fatalf("ListDataFiles: %v", err)
	}
	if len(files) != 0 {
		t.Errorf("expected 0 data files, got %d", len(files))
	}
}

func TestDoubleCloseIsSafe(t *testing.T) {
	dir := tempDir(t)
	cat, err := rocklake.Open(dir)
	if err != nil {
		t.Fatalf("Open: %v", err)
	}
	cat.Close()
	cat.Close() // second close must not panic
}
