package io.trickle.rocklake;

import java.io.File;
import java.io.InputStream;
import java.io.OutputStream;
import java.nio.file.Files;
import java.nio.file.StandardCopyOption;
import java.util.List;
import java.util.Map;

/**
 * JNI binding to the native RockLake C ABI.
 * 
 * This class handles loading the native library and declaring the JNI method stubs.
 * The actual implementations are provided by the native rocklake.so/rocklake.dll/rocklake.dylib.
 */
class RockLakeNative {
    private static boolean libraryLoaded = false;

    /**
     * Loads the native RockLake library.
     * 
     * Attempts to load from the JAR resource first (for packaged distributions),
     * then falls back to system library paths.
     */
    static synchronized void loadLibrary() {
        if (libraryLoaded) {
            return;
        }

        try {
            // Try to load from JAR resources first (for packaged distributions)
            String libName = getLibraryName();
            try {
                loadFromResource(libName);
            } catch (Exception e) {
                // Fall back to system library paths
                System.loadLibrary("rocklake");
            }
            libraryLoaded = true;
        } catch (UnsatisfiedLinkError e) {
            throw new ExceptionInInitializerError("Failed to load RockLake native library: " + e.getMessage());
        }
    }

    private static String getLibraryName() {
        String osName = System.getProperty("os.name").toLowerCase();
        String osArch = System.getProperty("os.arch").toLowerCase();
        String libExtension;
        
        if (osName.contains("windows")) {
            libExtension = ".dll";
        } else if (osName.contains("mac")) {
            libExtension = ".dylib";
        } else {
            libExtension = ".so";
        }
        
        // Map architecture names
        String arch;
        if (osArch.contains("amd64") || osArch.contains("x86_64")) {
            arch = "x86_64";
        } else if (osArch.contains("aarch64") || osArch.contains("arm64")) {
            arch = "aarch64";
        } else {
            arch = osArch;
        }
        
        String platform;
        if (osName.contains("windows")) {
            platform = "windows-" + arch;
        } else if (osName.contains("mac")) {
            platform = "macos-" + arch;
        } else if (osName.contains("linux")) {
            platform = "linux-" + arch;
        } else {
            platform = osName + "-" + arch;
        }
        
        return "rocklake-" + platform + libExtension;
    }

    private static void loadFromResource(String libName) throws Exception {
        String resourcePath = "/native/" + libName;
        try (InputStream in = RockLakeNative.class.getResourceAsStream(resourcePath)) {
            if (in == null) {
                throw new Exception("Native library not found in resources: " + resourcePath);
            }
            
            File tempFile = File.createTempFile("rocklake", 
                System.getProperty("os.name").toLowerCase().contains("windows") ? ".dll" : 
                System.getProperty("os.name").toLowerCase().contains("mac") ? ".dylib" : ".so");
            tempFile.deleteOnExit();
            
            try (OutputStream out = Files.newOutputStream(tempFile.toPath())) {
                byte[] buffer = new byte[8192];
                int read;
                while ((read = in.read(buffer)) != -1) {
                    out.write(buffer, 0, read);
                }
            }
            
            System.load(tempFile.getAbsolutePath());
        }
    }

    // ──────────────────────────────────────────────────────────────────────────
    // Native JNI method stubs
    // ──────────────────────────────────────────────────────────────────────────

    /**
     * Native: opens or creates a catalog.
     * Corresponds to rocklake_open_catalog in rocklake.h.
     */
    native static long openCatalog(String path, Map<String, String> options) throws RockLakeException;

    /**
     * Native: gets the current snapshot ID.
     * Corresponds to rocklake_get_snapshot in rocklake.h.
     */
    native static long getSnapshot(long handle) throws RockLakeException;

    /**
     * Native: lists data files for a table.
     * Corresponds to rocklake_list_data_files in rocklake.h.
     */
    native static List<DataFileRow> listDataFiles(long handle, String tableId, long snapshotId) throws RockLakeException;

    /**
     * Native: describes a table schema.
     * Corresponds to rocklake_describe_table in rocklake.h.
     */
    native static List<ColumnRow> describeTable(long handle, String tableId, long snapshotId) throws RockLakeException;

    /**
     * Native: creates a new snapshot.
     * Corresponds to rocklake_create_snapshot in rocklake.h.
     */
    native static long createSnapshot(long handle, String changes) throws RockLakeException;

    /**
     * Native: closes the catalog.
     * Corresponds to rocklake_close_catalog in rocklake.h.
     */
    native static void closeCatalog(long handle) throws RockLakeException;
}
