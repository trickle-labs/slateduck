package io.trickle.rocklake.examples;

import io.trickle.rocklake.RockLakeCatalog;
import io.trickle.rocklake.DataFileRow;
import io.trickle.rocklake.RockLakeException;

/**
 * Example: Using RockLake catalog with Apache Flink.
 * 
 * This is a SourceFunction stub demonstrating how to integrate RockLake
 * with Flink's source API for streaming reads of catalog updates.
 * 
 * In a production implementation, this would:
 * 1. Poll the RockLake catalog for new snapshots at configurable intervals
 * 2. Emit snapshot diffs as Flink SourceEvents
 * 3. Coordinate checkpoint state with RockLake pin/unpin operations
 */
public class FlinkCatalogSource {
    
    private final String catalogPath;
    private final String tableId;
    private final long pollIntervalMs;
    private RockLakeCatalog catalog;
    private long lastSnapshotSeen;

    /**
     * Creates a Flink source for RockLake catalog updates.
     * 
     * @param catalogPath the catalog path (e.g., "s3://bucket/catalog")
     * @param tableId the table ID to monitor (e.g., "my_schema.my_table")
     * @param pollIntervalMs how often to check for new snapshots (ms)
     */
    public FlinkCatalogSource(String catalogPath, String tableId, long pollIntervalMs) {
        this.catalogPath = catalogPath;
        this.tableId = tableId;
        this.pollIntervalMs = pollIntervalMs;
        this.lastSnapshotSeen = -1L;
    }

    /**
     * Opens the source and initializes the catalog connection.
     */
    public void open() throws RockLakeException {
        this.catalog = new RockLakeCatalog(catalogPath);
        this.lastSnapshotSeen = catalog.getSnapshot();
        System.out.println("FlinkCatalogSource opened. Current snapshot: " + lastSnapshotSeen);
    }

    /**
     * Polls the catalog for new snapshots and emits them as events.
     * 
     * This method would be called by Flink at each checkpoint interval.
     * In a real implementation, it would:
     * 1. Check the current snapshot
     * 2. If newer than lastSnapshotSeen, fetch snapshot diffs
     * 3. Emit them to the collector
     * 4. Update lastSnapshotSeen
     */
    public void run(SourceContext<SnapshotDiff> ctx) throws RockLakeException {
        while (true) {
            try {
                Thread.sleep(pollIntervalMs);
                
                long currentSnapshot = catalog.getSnapshot();
                if (currentSnapshot > lastSnapshotSeen) {
                    System.out.println("New snapshot detected: " + currentSnapshot + 
                                     " (was " + lastSnapshotSeen + ")");
                    
                    // Fetch data files for this snapshot
                    var dataFiles = catalog.listDataFiles(tableId, currentSnapshot);
                    
                    // Emit a snapshot diff event
                    SnapshotDiff diff = new SnapshotDiff(
                        lastSnapshotSeen,
                        currentSnapshot,
                        dataFiles
                    );
                    
                    synchronized (ctx.getCheckpointLock()) {
                        ctx.collect(diff);
                    }
                    
                    lastSnapshotSeen = currentSnapshot;
                }
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
                break;
            }
        }
    }

    /**
     * Closes the catalog connection.
     */
    public void close() throws RockLakeException {
        if (catalog != null) {
            catalog.close();
        }
    }

    /**
     * Represents a difference between two catalog snapshots.
     */
    public static class SnapshotDiff {
        public final long fromSnapshot;
        public final long toSnapshot;
        public final java.util.List<DataFileRow> newFiles;

        public SnapshotDiff(long fromSnapshot, long toSnapshot, 
                           java.util.List<DataFileRow> newFiles) {
            this.fromSnapshot = fromSnapshot;
            this.toSnapshot = toSnapshot;
            this.newFiles = newFiles;
        }

        @Override
        public String toString() {
            return String.format("SnapshotDiff(from=%d to=%d, files=%d)",
                fromSnapshot, toSnapshot, newFiles.size());
        }
    }

    public static void main(String[] args) throws RockLakeException {
        if (args.length < 1) {
            System.err.println("Usage: java FlinkCatalogSource <catalog-path> [table-id] [poll-interval-ms]");
            System.err.println("Example: java FlinkCatalogSource s3://my-bucket/catalog my_schema.my_table 5000");
            System.exit(1);
        }

        String catalogPath = args[0];
        String tableId = args.length > 1 ? args[1] : "my_schema.my_table";
        long pollIntervalMs = args.length > 2 ? Long.parseLong(args[2]) : 5000L;

        try (FlinkCatalogSource source = new FlinkCatalogSource(catalogPath, tableId, pollIntervalMs)) {
            source.open();
            
            System.out.println("Starting to poll for snapshot updates...");
            System.out.println("Catalog path: " + catalogPath);
            System.out.println("Table ID: " + tableId);
            System.out.println("Poll interval: " + pollIntervalMs + " ms");
            System.out.println("Press Ctrl+C to stop.\n");
            
            // Simulate polling for 30 seconds
            long startTime = System.currentTimeMillis();
            while (System.currentTimeMillis() - startTime < 30000) {
                source.run(new SimpleSourceContext());
            }
            
        } catch (RockLakeException e) {
            System.err.println("RockLake error: " + e.getMessage());
            e.printStackTrace();
            System.exit(1);
        }
    }

    /**
     * Simple mock SourceContext for testing.
     */
    private static class SimpleSourceContext implements SourceContext<SnapshotDiff> {
        @Override
        public void collect(SnapshotDiff element) {
            System.out.println("Emitted: " + element);
        }

        @Override
        public Object getCheckpointLock() {
            return this;
        }

        @Override
        public void close() {
        }

        // Other methods not needed for this stub
    }

    /**
     * Mock interface for Flink SourceContext.
     * In a real Flink integration, this would be org.apache.flink.streaming.api.functions.source.SourceContext
     */
    private interface SourceContext<T> extends AutoCloseable {
        void collect(T element);
        Object getCheckpointLock();
        void close();
    }
}
