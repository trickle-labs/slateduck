package io.trickle.rocklake.examples;

import io.trickle.rocklake.RockLakeCatalog;
import io.trickle.rocklake.DataFileRow;
import io.trickle.rocklake.RockLakeException;
import org.apache.spark.sql.SparkSession;
import org.apache.spark.sql.DataFrame;

/**
 * Example: Using RockLake catalog with Apache Spark 3.5.
 * 
 * This example demonstrates how to:
 * 1. Open a RockLake catalog
 * 2. List data files for a table
 * 3. Register Parquet files with Spark
 * 4. Query them using DataFrame API
 * 
 * Usage:
 * ```
 * SparkCatalogReader reader = new SparkCatalogReader("s3://my-bucket/catalog");
 * DataFrame df = reader.readTable("my_spark.my_table", spark);
 * df.show();
 * ```
 */
public class SparkCatalogReader implements AutoCloseable {
    private final RockLakeCatalog catalog;
    private final String catalogPath;

    public SparkCatalogReader(String catalogPath) throws RockLakeException {
        this.catalogPath = catalogPath;
        this.catalog = new RockLakeCatalog(catalogPath);
    }

    /**
     * Reads a table from the RockLake catalog into a Spark DataFrame.
     * 
     * @param tableId the table ID (e.g., "schema.table")
     * @param spark the SparkSession
     * @return a DataFrame containing the table data
     * @throws RockLakeException if reading fails
     */
    public DataFrame readTable(String tableId, SparkSession spark) throws RockLakeException {
        // Get the current snapshot
        long snapshot = catalog.getSnapshot();
        System.out.println("Reading table '" + tableId + "' from snapshot " + snapshot);

        // List all data files for this table
        var dataFiles = catalog.listDataFiles(tableId, snapshot);
        System.out.println("Found " + dataFiles.size() + " data files:");

        // Build a list of file paths
        StringBuilder fileList = new StringBuilder();
        long totalRecords = 0;

        for (DataFileRow file : dataFiles) {
            System.out.println("  - " + file.filePath + " (" + file.recordCount + " records)");
            if (fileList.length() > 0) {
                fileList.append(",");
            }
            fileList.append(file.filePath);
            totalRecords += file.recordCount;
        }

        System.out.println("Total records: " + totalRecords);

        // Register with Spark using the Parquet reader
        if (fileList.length() > 0) {
            return spark.read().parquet(fileList.toString().split(","));
        } else {
            // Return empty DataFrame with the schema
            System.out.println("No data files found; returning empty DataFrame");
            var schema = catalog.describeTable(tableId, snapshot);
            // In a real implementation, we'd convert schema to StructType
            // For now, return an empty DataFrame
            return spark.createDataFrame(java.util.Collections.emptyList(), "id long");
        }
    }

    /**
     * Gets the catalog path.
     */
    public String getCatalogPath() {
        return catalogPath;
    }

    @Override
    public void close() throws RockLakeException {
        catalog.close();
    }

    public static void main(String[] args) throws RockLakeException {
        if (args.length < 1) {
            System.err.println("Usage: java SparkCatalogReader <catalog-path> [table-id]");
            System.err.println("Example: java SparkCatalogReader s3://my-bucket/catalog my_schema.my_table");
            System.exit(1);
        }

        String catalogPath = args[0];
        String tableId = args.length > 1 ? args[1] : "my_schema.my_table";

        try (SparkSession spark = SparkSession.builder()
                .appName("RockLake-Spark-Reader")
                .master("local[*]")
                .getOrCreate();
             SparkCatalogReader reader = new SparkCatalogReader(catalogPath)) {

            System.out.println("Opening RockLake catalog at: " + catalogPath);
            
            // Read the table
            DataFrame df = reader.readTable(tableId, spark);
            
            System.out.println("Schema:");
            df.printSchema();
            
            System.out.println("Data:");
            df.show();
            
            System.out.println("Row count: " + df.count());
            
        } catch (RockLakeException e) {
            System.err.println("RockLake error: " + e.getMessage());
            e.printStackTrace();
            System.exit(1);
        }
    }
}
