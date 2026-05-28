// RockLake Node.js — 10-line example.
//
// Usage:
//   npm install @rocklake/client
//   node examples/quickstart.js /path/to/catalog

const { Catalog } = require('@rocklake/client');

const path = process.argv[2] || '/tmp/demo-catalog';
const cat = Catalog.open(path);
const snap = cat.snapshotId();
const schemas = cat.listSchemas(snap);
console.log(`Snapshot ${snap}: ${schemas.length} schema(s)`);
cat.close();
