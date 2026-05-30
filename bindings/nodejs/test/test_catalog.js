// Node.js integration tests for @rocklake/client
// Run: node --test test/test_catalog.js
// Requires the native module to be built first: napi build --platform

const { test } = require('node:test');
const assert = require('node:assert');
const os = require('os');
const fs = require('fs');
const path = require('path');

let Catalog;
try {
  ({ Catalog } = require('../index'));
} catch (e) {
  // Module not built yet — skip all tests
  console.warn('Skipping Node.js tests: native module not built yet.');
  process.exit(0);
}

function tmpDir() {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'rocklake-node-test-'));
}

test('open and close', () => {
  const dir = tmpDir();
  const cat = Catalog.open(dir);
  cat.close();
});

test('snapshotId returns 0 for fresh catalog', () => {
  const dir = tmpDir();
  const cat = Catalog.open(dir);
  // v0.46.0: snapshotId() returns BigInt
  assert.strictEqual(cat.snapshotId(), 0n);
  cat.close();
});

test('listSchemas returns empty array for fresh catalog', () => {
  const dir = tmpDir();
  const cat = Catalog.open(dir);
  const schemas = cat.listSchemas(0n);
  assert.deepStrictEqual(schemas, []);
  cat.close();
});

test('listTables returns empty array for fresh catalog', () => {
  const dir = tmpDir();
  const cat = Catalog.open(dir);
  const tables = cat.listTables(1n, 0n);
  assert.deepStrictEqual(tables, []);
  cat.close();
});

test('listDataFiles returns empty array for fresh catalog', () => {
  const dir = tmpDir();
  const cat = Catalog.open(dir);
  const files = cat.listDataFiles(1n, 0n);
  assert.deepStrictEqual(files, []);
  cat.close();
});

test('double close is safe', () => {
  const dir = tmpDir();
  const cat = Catalog.open(dir);
  cat.close();
  cat.close(); // must not throw
});

// v0.46.0: BigInt ID round-trip fidelity test.
// Verifies that IDs above u32::MAX are not silently truncated.
test('snapshotId returned as BigInt (type check)', () => {
  const dir = tmpDir();
  const cat = Catalog.open(dir);
  const id = cat.snapshotId();
  // Must be BigInt, not a plain number.
  assert.strictEqual(typeof id, 'bigint', `snapshotId() should return bigint, got ${typeof id}`);
  cat.close();
});

test('listSchemas accepts BigInt snapshotId', () => {
  const dir = tmpDir();
  const cat = Catalog.open(dir);
  // 0n is BigInt — must not throw a type error.
  const schemas = cat.listSchemas(0n);
  assert.ok(Array.isArray(schemas));
  cat.close();
});
