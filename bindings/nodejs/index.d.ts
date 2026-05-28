// TypeScript declarations for @rocklake/client

export interface Snapshot {
  snapshotId: number;
}

export interface Schema {
  schemaId: number;
  schemaName: string;
}

export interface Table {
  tableId: number;
  schemaId: number;
  tableName: string;
}

export interface DataFile {
  dataFileId: number;
  tableId: number;
  path: string;
  fileFormat: string;
  rowCount: number;
  fileSizeBytes: number;
  snapshotId: number;
}

export declare class Catalog {
  /** Open a catalog at *uri*. */
  static open(uri: string): Catalog;
  /** Return the current snapshot ID (0 = empty catalog). */
  snapshotId(): number;
  /** Return the current snapshot. */
  currentSnapshot(): Snapshot;
  /** List schemas at *snapshotId* (0 = latest). */
  listSchemas(snapshotId: number): Schema[];
  /** List tables in *schemaId* at *snapshotId*. */
  listTables(schemaId: number, snapshotId: number): Table[];
  /** List data files for *tableId* at *snapshotId*. */
  listDataFiles(tableId: number, snapshotId: number): DataFile[];
  /** Close the catalog. */
  close(): void;
}
