package io.trickle.rocklake;

/**
 * Represents a column in a RockLake table.
 */
public class ColumnRow {
    public final int columnId;
    public final String columnName;
    public final String dataType;
    public final boolean nullable;
    public final String defaultValue;
    
    public ColumnRow(int columnId, String columnName, String dataType, boolean nullable, String defaultValue) {
        this.columnId = columnId;
        this.columnName = columnName;
        this.dataType = dataType;
        this.nullable = nullable;
        this.defaultValue = defaultValue;
    }

    @Override
    public String toString() {
        return String.format("ColumnRow(id=%d, name=%s, type=%s, nullable=%b)",
            columnId, columnName, dataType, nullable);
    }
}
