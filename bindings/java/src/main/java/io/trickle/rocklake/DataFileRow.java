package io.trickle.rocklake;

/**
 * Represents a data file in the RockLake catalog.
 */
public class DataFileRow {
    public final long fileId;
    public final String filePath;
    public final long fileSize;
    public final long recordCount;
    public final long minRowId;
    public final long maxRowId;
    public final long createdAtSnapshot;
    
    public DataFileRow(long fileId, String filePath, long fileSize, long recordCount,
                      long minRowId, long maxRowId, long createdAtSnapshot) {
        this.fileId = fileId;
        this.filePath = filePath;
        this.fileSize = fileSize;
        this.recordCount = recordCount;
        this.minRowId = minRowId;
        this.maxRowId = maxRowId;
        this.createdAtSnapshot = createdAtSnapshot;
    }

    @Override
    public String toString() {
        return String.format("DataFileRow(id=%d, path=%s, size=%d, records=%d, rows=%d..%d, created=%d)",
            fileId, filePath, fileSize, recordCount, minRowId, maxRowId, createdAtSnapshot);
    }
}
