# JVM Bindings — Java & Kotlin APIs

> RockLake JVM bindings enable native integration with Apache Spark, Flink, and other JVM-based analytics engines without a PG-wire sidecar.

## Overview

The `rocklake-java` Maven artifact exposes the complete `rocklake.h` C ABI via JNI, providing:

- **Type-safe Java API** (`RockLakeCatalog`) for catalog operations
- **Kotlin coroutine wrapper** (`RockLakeCatalogAsync`) for async operations
- **Spark 3.5 integration** example for reading Parquet files into DataFrames
- **Flink streaming source** stub for snapshot-diff-driven ingestion
- **Multi-platform native libraries** (Linux x86-64/aarch64, macOS arm64, Windows x86-64)

---

## Installation

### Maven

Add to your `pom.xml`:

```xml
<dependency>
    <groupId>io.trickle</groupId>
    <artifactId>rocklake-java</artifactId>
    <version>0.44.0</version>
</dependency>
```

### Gradle

Add to your `build.gradle` or `build.gradle.kts`:

```gradle
dependencies {
    implementation("io.trickle:rocklake-java:0.44.0")
}
```

The Maven artifact includes native libraries for all supported platforms. The `RockLakeNative` class automatically loads the correct library at runtime.

---

## Java API

### Basic Usage

```java
import io.trickle.rocklake.RockLakeCatalog;
import io.trickle.rocklake.DataFileRow;

public class Example {
    public static void main(String[] args) throws Exception {
        // Open a catalog
        try (RockLakeCatalog catalog = new RockLakeCatalog("s3://my-bucket/catalog")) {
            // Get the current snapshot
            long snapshot = catalog.getSnapshot();
            System.out.println("Current snapshot: " + snapshot);

            // List data files for a table
            var files = catalog.listDataFiles("my_schema.my_table");
            for (DataFileRow file : files) {
                System.out.println("File: " + file.filePath + " (" + file.recordCount + " records)");
            }

            // Describe table schema
            var columns = catalog.describeTable("my_schema.my_table");
            for (var col : columns) {
                System.out.println("Column: " + col.columnName + " (" + col.dataType + ")");
            }
        }
    }
}
```

### Core API Reference

#### `RockLakeCatalog`

**Constructor:**
```java
// With default options
RockLakeCatalog(String path) throws RockLakeException;

// With custom options
RockLakeCatalog(String path, Map<String, String> options) throws RockLakeException;
```

**Methods:**
```java
// Snapshot management
long getSnapshot() throws RockLakeException;
long createSnapshot(String changes) throws RockLakeException;

// Data file operations
List<DataFileRow> listDataFiles(String tableId) throws RockLakeException;
List<DataFileRow> listDataFiles(String tableId, long snapshotId) throws RockLakeException;

// Schema discovery
List<ColumnRow> describeTable(String tableId) throws RockLakeException;
List<ColumnRow> describeTable(String tableId, long snapshotId) throws RockLakeException;

// Lifecycle
void close() throws RockLakeException;
boolean isOpen();
String getPath();
```

#### `DataFileRow`

Represents a single data file in the catalog:

```java
public class DataFileRow {
    public final long fileId;
    public final String filePath;
    public final long fileSize;
    public final long recordCount;
    public final long minRowId;
    public final long maxRowId;
    public final long createdAtSnapshot;
}
```

#### `ColumnRow`

Represents a column in a table:

```java
public class ColumnRow {
    public final int columnId;
    public final String columnName;
    public final String dataType;
    public final boolean nullable;
    public final String defaultValue;
}
```

---

## Kotlin API

### Coroutine Support

The `RockLakeCatalogAsync` wrapper provides Kotlin coroutine support for non-blocking operations:

```kotlin
import io.trickle.rocklake.RockLakeCatalog
import io.trickle.rocklake.async
import kotlinx.coroutines.runBlocking

fun main() = runBlocking {
    RockLakeCatalog("s3://my-bucket/catalog").async().use { async ->
        // All operations are suspended on Dispatchers.IO
        val snapshot = async.getSnapshot()
        println("Current snapshot: $snapshot")

        val files = async.listDataFiles("my_schema.my_table")
        files.forEach { file ->
            println("File: ${file.filePath} (${file.recordCount} records)")
        }

        val columns = async.describeTable("my_schema.my_table")
        columns.forEach { col ->
            println("Column: ${col.columnName} (${col.dataType})")
        }
    }
}
```

### Extension Functions

**`async()`** — Creates an async wrapper:
```kotlin
val catalog = RockLakeCatalog("...")
val async = catalog.async()
```

**`use { block }`** — Resource-safe async operations:
```kotlin
async.use { a ->
    val snapshot = a.getSnapshot()
    // catalog is automatically closed
}
```

---

## Spark Integration

### Example: Reading a Table into a DataFrame

```java
import io.trickle.rocklake.examples.SparkCatalogReader;
import org.apache.spark.sql.SparkSession;

public class SparkExample {
    public static void main(String[] args) throws Exception {
        SparkSession spark = SparkSession.builder()
            .appName("RockLake-Spark")
            .master("local[*]")
            .getOrCreate();

        try (SparkCatalogReader reader = new SparkCatalogReader("s3://my-bucket/catalog")) {
            // Read a table from RockLake into a Spark DataFrame
            var df = reader.readTable("my_schema.my_table", spark);
            
            df.printSchema();
            df.show();
            System.out.println("Row count: " + df.count());
        }
    }
}
```

### Implementation Details

The `SparkCatalogReader` class:
1. Opens the RockLake catalog
2. Fetches all data files for the target table
3. Builds a comma-separated list of Parquet paths
4. Uses Spark's `read().parquet()` API to load them
5. Returns a unified DataFrame

This approach works seamlessly with Spark's native Parquet reader and supports all Spark operations (filtering, projection, aggregation, etc.).

---

## Flink Integration

### Example: Streaming Snapshot Diffs

```java
import io.trickle.rocklake.examples.FlinkCatalogSource;

public class FlinkExample {
    public static void main(String[] args) throws Exception {
        // This is a conceptual example; real Flink integration would use:
        // StreamExecutionEnvironment env = StreamExecutionEnvironment.getExecutionEnvironment();
        // env.addSource(new FlinkCatalogSource("s3://...", "table_id", 5000L))
        //     .print();
        // env.execute("RockLake Flink Source");
        
        FlinkCatalogSource source = new FlinkCatalogSource(
            "s3://my-bucket/catalog",
            "my_schema.my_table",
            5000  // poll interval in ms
        );
        
        source.open();
        System.out.println("Flink source ready to stream snapshot diffs");
    }
}
```

### Implementation Details

The `FlinkCatalogSource` class:
1. Polls the RockLake catalog at configurable intervals
2. Detects new snapshots compared to the last checkpoint
3. Emits `SnapshotDiff` events containing new data files
4. Coordinates with Flink's checkpoint mechanism for exactly-once semantics

---

## Configuration

### Environment Variables

The JVM bindings respect the following environment variables:

| Variable | Purpose |
|----------|---------|
| `ROCKLAKE_CATALOG_URI` | Default catalog path if not specified in code |
| `ROCKLAKE_OBJECT_STORE` | Backend: `local`, `s3`, `gcs`, `azure` |
| `AWS_ACCESS_KEY_ID` | AWS credentials for S3 (if `object_store=s3`) |
| `AWS_SECRET_ACCESS_KEY` | AWS credentials for S3 |
| `AWS_REGION` | AWS region for S3 (default: `us-east-1`) |
| `GOOGLE_APPLICATION_CREDENTIALS` | Path to GCS service account JSON |
| `AZURE_STORAGE_ACCOUNT_NAME` | Azure storage account |
| `AZURE_STORAGE_ACCOUNT_KEY` | Azure storage key |

### Programmatic Options

Pass options to the constructor:

```java
Map<String, String> options = new HashMap<>();
options.put("object_store", "s3");
options.put("aws_region", "eu-west-1");
options.put("encryption", "sse");

RockLakeCatalog catalog = new RockLakeCatalog("s3://bucket/catalog", options);
```

---

## Error Handling

All API methods throw `RockLakeException` on failure:

```java
try {
    catalog.getSnapshot();
} catch (RockLakeException e) {
    System.err.println("Failed to get snapshot: " + e.getMessage());
    e.printStackTrace();
}
```

### Common Errors

| Error | Cause | Solution |
|-------|-------|----------|
| `Catalog is closed` | Attempting operation on closed catalog | Check `isOpen()` before operations |
| `Failed to load RockLake native library` | Native library not found for platform | Verify Maven artifact includes your platform |
| `Cannot find table` | Table ID does not exist | Use `listTables()` to discover available tables |
| `Invalid snapshot ID` | Snapshot does not exist | Use `getSnapshot()` to get current snapshot ID |

---

## Supported Platforms

The Maven artifact includes native libraries for:

| Platform | Architecture | Filename |
|----------|-------------|----------|
| Linux | x86-64 | `librocklake-linux-x86_64.so` |
| Linux | aarch64 | `librocklake-linux-aarch64.so` |
| macOS | arm64 | `librocklake-macos-arm64.dylib` |
| Windows | x86-64 | `rocklake-windows-x86_64.dll` |

---

## Building from Source

### Prerequisites

- Java Development Kit (JDK) 21 LTS or later
- Rust toolchain (for building native libraries)
- Gradle or Maven

### Build Steps

```bash
# Clone the repository
git clone https://github.com/trickle-labs/rocklake.git
cd rocklake/bindings/java

# Build the Java bindings and native libraries
./gradlew build

# Run tests
./gradlew test

# Publish to local Maven repository
./gradlew publishToMavenLocal
```

### Building Native Libraries Only

If you already have a compiled native library, skip the Cargo build step:

```bash
./gradlew build -x buildNativeLibrary
```

---

## See Also

- [C API Reference](../reference/c-api.md)
- [Spark Integration Guide](https://github.com/trickle-labs/rocklake/tree/main/bindings/java/examples/SparkCatalogReader.java)
- [Flink Integration Guide](https://github.com/trickle-labs/rocklake/tree/main/bindings/java/examples/FlinkCatalogSource.java)
- [Python Bindings](https://github.com/trickle-labs/rocklake/tree/main/bindings/python)
- [Go Bindings](https://github.com/trickle-labs/rocklake/tree/main/bindings/go)
- [Node.js Bindings](https://github.com/trickle-labs/rocklake/tree/main/bindings/nodejs)
