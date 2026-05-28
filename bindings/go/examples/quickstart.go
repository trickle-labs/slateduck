// RockLake Go — 10-line example.
//
// Usage:
//   go run examples/quickstart.go /path/to/catalog
package main

import (
	"fmt"
	"log"
	"os"

	rocklake "github.com/trickle-labs/rocklake-go"
)

func main() {
	path := "/tmp/demo-catalog"
	if len(os.Args) > 1 {
		path = os.Args[1]
	}
	cat, err := rocklake.Open(path)
	if err != nil {
		log.Fatal(err)
	}
	defer cat.Close()
	snap, _ := cat.SnapshotID()
	schemas, _ := cat.ListSchemas(snap)
	fmt.Printf("Snapshot %d: %d schema(s)\n", snap, len(schemas))
}
