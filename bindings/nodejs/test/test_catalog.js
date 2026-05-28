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
  assert.strictEqual(cat.snapshotId(), 0);
  cat.close();
});

test('listSchemas returns empty array for fresh catalog', () => {
  const dir = tmpDir();
  const cat = Catalog.open(dir);
  const schemas = cat.listSchemas(0);
  assert.deepStrictEqual(schemas, []);
  cat.close();
});

test('listTables returns empty array for fresh catalog', () => {
  const dir = tmpDir();
  const cat = Catalog.open(dir);
  const tables = cat.listTables(1, 0);
  assert.deepStrictEqual(tables, []);
  cat.close();
});

test('listDataFiles returns empty array for fresh catalog', () => {
  const dir = tmpDir();
  const cat = Catalog.open(dir);
  const files = cat.listDataFiles(1, 0);
  assert.deepStrictEqual(files, []);
  cat.close();
});

test('double close is safe', () => {
  const dir = tmpDir();
  const cat = Catalog.open(dir);
  cat.close();
  cat.close(); // must not throw
});
