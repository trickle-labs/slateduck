package io.trickle.rocklake;

/**
 * Exception thrown when a RockLake operation fails.
 */
public class RockLakeException extends Exception {
    public RockLakeException(String message) {
        super(message);
    }

    public RockLakeException(String message, Throwable cause) {
        super(message, cause);
    }
}
