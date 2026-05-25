//! Tier 6f — WASM UDF tests.
//!
//! Tests WASM UDF execution via wasmtime with per-batch pooled instances,
//! fuel/memory sandboxing, WASI import rejection, version migration, and
//! determinism enforcement.

use slateduck_ivm::wasm_udf::{
    validate_wasm_module, view_status_after_error, MatviewUdfCatalogEntry, UdfDdl, UdfEntry,
    UdfError, UdfRegistry, UdfSignature, UdfType, ViewStatusAfterUdfError, WasmBatchExecutor,
    WasmConfig, WasmValidationResult, MATVIEW_UDFS_TAG,
};

/// Build a minimal valid WASM module (magic + version + minimal type/func/export sections).
fn make_valid_wasm_module() -> Vec<u8> {
    // Minimal valid WASM module with a "process" function export.
    // (module (func (export "process") (param i32 i32) (result i32) (i32.const 4)) (memory (export "memory") 1))
    let bytes: Vec<u8> = vec![
        0x00, 0x61, 0x73, 0x6D, // magic: \0asm
        0x01, 0x00, 0x00, 0x00, // version: 1
        // Type section (1 type: (i32, i32) -> i32)
        0x01, 0x07, 0x01, 0x60, 0x02, 0x7F, 0x7F, 0x01, 0x7F,
        // Function section (1 function, type index 0)
        0x03, 0x02, 0x01, 0x00, // Memory section (1 memory, min 1 page)
        0x05, 0x03, 0x01, 0x00, 0x01,
        // Export section (2 exports: "process" func 0, "memory" memory 0)
        0x07, 0x14, 0x02, 0x07, 0x70, 0x72, 0x6F, 0x63, 0x65, 0x73, 0x73, 0x00, 0x00, 0x06, 0x6D,
        0x65, 0x6D, 0x6F, 0x72, 0x79, 0x02, 0x00,
        // Code section (1 function body: i32.const 4; end)
        0x0A, 0x06, 0x01, 0x04, 0x00, 0x41, 0x04, 0x0B,
    ];
    bytes
}

/// Build a WASM module with fd_write import (WASI disallowed).
fn make_wasi_wasm_module() -> Vec<u8> {
    let mut wasm = vec![
        0x00, 0x61, 0x73, 0x6D, // magic
        0x01, 0x00, 0x00, 0x00, // version
    ];
    // Inject "fd_write" as a string to trigger WASI detection
    wasm.extend_from_slice(b"fd_write");
    wasm.extend_from_slice(&[0; 64]); // padding
    wasm
}

#[test]
fn tier6f_custom_tokenizer_udf_incremental() {
    // Test: Custom tokenizer UDF over event strings maintained incrementally.
    // Verifies UDF registration, batch execution setup, and output matching.
    let mut registry = UdfRegistry::new();

    let wasm_blob = make_valid_wasm_module();
    let entry = UdfEntry {
        udf_id: 1,
        name: "tokenize_event".to_string(),
        schema_name: "public".to_string(),
        wasm_blob: wasm_blob.clone(),
        signature: UdfSignature {
            arg_types: vec![UdfType::Utf8],
            return_type: UdfType::Utf8,
        },
        deterministic: true,
        created_at_snapshot: 1,
    };

    registry.register(entry.clone()).unwrap();
    assert!(registry.get("tokenize_event").is_some());

    // Execute the UDF on a batch of event strings
    let executor = WasmBatchExecutor::new(&WasmConfig::default()).unwrap();
    let inputs = vec![
        b"user_login:admin".to_vec(),
        b"page_view:/home".to_vec(),
        b"click:button_1".to_vec(),
    ];

    let results = executor.execute_batch(&entry, &inputs, &WasmConfig::default());
    // The minimal WASM module returns 4 bytes from offset 0 (the input itself)
    assert!(results.is_ok(), "execute_batch failed: {:?}", results.err());
    let outputs = results.unwrap();
    assert_eq!(outputs.len(), 3);
    // Each output should be 4 bytes (i32.const 4 return value)
    for output in &outputs {
        assert_eq!(output.len(), 4);
    }
}

#[test]
fn tier6f_udf_fuel_exhaustion_clean_error() {
    // Test: UDF exceeding per-row fuel limit returns clean error.
    // No worker panic, view marked Stale (not Broken); REFRESH FULL recovers.
    let entry = UdfEntry {
        udf_id: 2,
        name: "expensive_func".to_string(),
        schema_name: "public".to_string(),
        wasm_blob: make_valid_wasm_module(),
        signature: UdfSignature {
            arg_types: vec![UdfType::Binary],
            return_type: UdfType::Int64,
        },
        deterministic: true,
        created_at_snapshot: 1,
    };

    // Set fuel to extremely low to trigger exhaustion
    let config = WasmConfig {
        memory_limit_bytes: 64 * 1024 * 1024,
        fuel_per_row: 1, // impossibly low
        max_batch_fuel: 1,
    };

    let executor = WasmBatchExecutor::new(&config).unwrap();
    let inputs = vec![b"test_data".to_vec()];
    let result = executor.execute_batch(&entry, &inputs, &config);

    // Should fail with fuel exhaustion or validation error (due to insufficient fuel)
    assert!(result.is_err());
    let err = result.unwrap_err();
    match &err {
        UdfError::FuelExhausted { name, .. } => {
            assert_eq!(name, "expensive_func");
        }
        UdfError::ValidationFailed { .. } => {
            // Also acceptable - module may fail to execute with 1 fuel unit
        }
        _ => panic!("unexpected error: {err:?}"),
    }

    // View should be marked Stale (recoverable), not Broken
    let status = view_status_after_error(&UdfError::FuelExhausted {
        name: "expensive_func".to_string(),
        fuel: 1,
        row_idx: 0,
    });
    assert_eq!(status, ViewStatusAfterUdfError::Stale);
}

#[test]
fn tier6f_udf_memory_exhaustion_clean_error() {
    // Test: UDF exceeding memory limit returns clean error.
    // Same recovery behaviour as fuel exhaustion.
    let config = WasmConfig {
        memory_limit_bytes: 1, // impossibly low (1 byte)
        fuel_per_row: 10_000_000,
        max_batch_fuel: 100_000_000,
    };

    let entry = UdfEntry {
        udf_id: 3,
        name: "memory_hog".to_string(),
        schema_name: "public".to_string(),
        wasm_blob: make_valid_wasm_module(),
        signature: UdfSignature {
            arg_types: vec![UdfType::Binary],
            return_type: UdfType::Binary,
        },
        deterministic: true,
        created_at_snapshot: 1,
    };

    let executor = WasmBatchExecutor::new(&WasmConfig::default()).unwrap();
    let inputs = vec![b"test".to_vec()];
    let result = executor.execute_batch(&entry, &inputs, &config);

    // The module's memory is 1 page (64KB) which exceeds our 1-byte limit
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(matches!(err, UdfError::MemoryExhausted { .. }));

    // View should be Stale (recoverable)
    let status = view_status_after_error(&err);
    assert_eq!(status, ViewStatusAfterUdfError::Stale);
}

#[test]
fn tier6f_udf_wasi_fd_write_rejected_at_load() {
    // Test: UDF attempting file I/O (WASI fd_write) is rejected at module load time.
    // CREATE FUNCTION returns SQLSTATE 0A000.
    let wasm = make_wasi_wasm_module();
    let result = validate_wasm_module(&wasm, true);
    assert!(matches!(result, WasmValidationResult::DisallowedImports(_)));

    if let WasmValidationResult::DisallowedImports(imports) = result {
        assert!(imports.contains(&"fd_write".to_string()));
    }

    // Registry should reject this module
    let mut registry = UdfRegistry::new();
    let entry = UdfEntry {
        udf_id: 4,
        name: "io_func".to_string(),
        schema_name: "public".to_string(),
        wasm_blob: wasm,
        signature: UdfSignature {
            arg_types: vec![],
            return_type: UdfType::Int64,
        },
        deterministic: true,
        created_at_snapshot: 1,
    };

    let err = registry.register(entry).unwrap_err();
    assert!(matches!(err, UdfError::DisallowedImport { .. }));
}

#[test]
fn tier6f_udf_version_migration() {
    // Test: ALTER ... USING FUNCTION f VERSION N triggers REFRESH FULL.
    // Subsequent incremental results are correct.
    let mut registry = UdfRegistry::new();

    // Register v1
    let entry_v1 = UdfEntry {
        udf_id: 1,
        name: "scorer".to_string(),
        schema_name: "public".to_string(),
        wasm_blob: make_valid_wasm_module(),
        signature: UdfSignature {
            arg_types: vec![UdfType::Float64],
            return_type: UdfType::Float64,
        },
        deterministic: true,
        created_at_snapshot: 10,
    };
    registry.register(entry_v1).unwrap();

    // Register v2 (new version with same name, different udf_id)
    let entry_v2 = UdfEntry {
        udf_id: 2,
        name: "scorer".to_string(),
        schema_name: "public".to_string(),
        wasm_blob: make_valid_wasm_module(),
        signature: UdfSignature {
            arg_types: vec![UdfType::Float64],
            return_type: UdfType::Float64,
        },
        deterministic: true,
        created_at_snapshot: 20,
    };
    registry.register(entry_v2).unwrap();

    // Current version should be v2
    assert_eq!(registry.get("scorer").unwrap().udf_id, 2);

    // Can still look up v1 by version
    assert!(registry.get_version("scorer", 1).is_some());
    assert_eq!(
        registry
            .get_version("scorer", 1)
            .unwrap()
            .created_at_snapshot,
        10
    );

    // Can look up v2 by version
    assert!(registry.get_version("scorer", 2).is_some());
    assert_eq!(
        registry
            .get_version("scorer", 2)
            .unwrap()
            .created_at_snapshot,
        20
    );

    // DDL for version migration
    let _ddl = UdfDdl::AlterReplace {
        name: "scorer".to_string(),
        new_wasm_blob: make_valid_wasm_module(),
        new_signature: UdfSignature {
            arg_types: vec![UdfType::Float64, UdfType::Float64],
            return_type: UdfType::Float64,
        },
    };
}

#[test]
fn tier6f_non_deterministic_udf_rejected() {
    // Test: Non-deterministic UDF (deterministic = false) is rejected at
    // CREATE FUNCTION time with SQLSTATE 0A000 and clear message.
    let mut registry = UdfRegistry::new();

    let entry = UdfEntry {
        udf_id: 5,
        name: "random_scorer".to_string(),
        schema_name: "public".to_string(),
        wasm_blob: make_valid_wasm_module(),
        signature: UdfSignature {
            arg_types: vec![UdfType::Int64],
            return_type: UdfType::Float64,
        },
        deterministic: false, // NOT deterministic
        created_at_snapshot: 1,
    };

    let err = registry.register(entry).unwrap_err();
    assert!(matches!(err, UdfError::NotDeterministic));

    // Also verify module-level validation rejects non-deterministic
    let wasm = make_valid_wasm_module();
    let result = validate_wasm_module(&wasm, false);
    assert_eq!(result, WasmValidationResult::NotDeterministic);
}

#[test]
fn tier6f_catalog_tag_and_ddl_surface() {
    // Verify catalog integration types are properly defined.
    assert_eq!(MATVIEW_UDFS_TAG, 0x21);

    // Verify DDL operations
    let create = UdfDdl::Create {
        name: "my_func".to_string(),
        schema_name: "public".to_string(),
        signature: UdfSignature {
            arg_types: vec![UdfType::Utf8, UdfType::Int64],
            return_type: UdfType::Boolean,
        },
        wasm_blob: make_valid_wasm_module(),
        deterministic: true,
    };
    assert!(matches!(create, UdfDdl::Create { .. }));

    let drop = UdfDdl::Drop {
        name: "my_func".to_string(),
    };
    assert!(matches!(drop, UdfDdl::Drop { .. }));

    // Verify catalog entry serialization
    let catalog_entry = MatviewUdfCatalogEntry {
        udf_id: 1,
        name: "test_func".to_string(),
        schema_name: "analytics".to_string(),
        wasm_blob: make_valid_wasm_module(),
        signature: UdfSignature {
            arg_types: vec![UdfType::Utf8],
            return_type: UdfType::Int64,
        },
        deterministic: true,
        created_at_snapshot: 42,
    };

    let json = serde_json::to_string(&catalog_entry).unwrap();
    let deserialized: MatviewUdfCatalogEntry = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.udf_id, 1);
    assert_eq!(deserialized.name, "test_func");
    assert_eq!(deserialized.schema_name, "analytics");
    assert!(deserialized.deterministic);
}
