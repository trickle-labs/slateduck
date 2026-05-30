//! v0.42.0 — Performance Benchmarks & Cost Analysis integration tests.
//!
//! Validates JSON benchmark deliverables and key deliverable artifacts.
//! Heavy catalog correctness is covered by the existing test suites;
//! these tests focus on the v0.42.0-specific deliverables.
//!
//! Test inventory (5 tests):
//! 1. benchmark_json_deliverable_valid
//! 2. baseline_json_thresholds_consistent
//! 3. update_benchmark_script_exists
//! 4. s3_express_validation_doc_present
//! 5. cost_analysis_doc_has_cost_per_operation_table

use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

#[test]
fn benchmark_json_deliverable_valid() {
    let path = workspace_root().join("benchmarks/v0.42-catalog-bench.json");
    assert!(
        path.exists(),
        "benchmarks/v0.42-catalog-bench.json must exist"
    );

    let data: serde_json::Value =
        serde_json::from_reader(std::fs::File::open(&path).unwrap()).unwrap();

    for key in &[
        "benchmark",
        "version",
        "measurements",
        "regression_thresholds",
    ] {
        assert!(
            data.get(key).is_some(),
            "v0.42-catalog-bench.json must have key '{key}'"
        );
    }

    let version = data["version"].as_str().unwrap_or("");
    assert!(
        version.starts_with("0.42"),
        "benchmark version must be 0.42.x, got '{version}'"
    );

    let measurements = data["measurements"].as_object().unwrap();
    for key in &[
        "get_current_snapshot_warm",
        "get_current_snapshot_cold",
        "list_data_files_100",
        "list_data_files_10k",
        "list_data_files_100k",
        "describe_table_100col",
        "create_snapshot_1_file",
        "create_snapshot_100_files",
        "prune_files_100k",
        "concurrent_readers_16",
    ] {
        assert!(
            measurements.contains_key(*key),
            "measurements must contain '{key}'"
        );
    }

    let thresholds = data["regression_thresholds"].as_object().unwrap();
    for key in &[
        "get_current_snapshot_warm",
        "list_data_files_100",
        "list_data_files_10k",
        "describe_table_100col",
        "create_snapshot_1_file",
        "create_snapshot_100_files",
        "prune_files_100k",
    ] {
        assert!(
            thresholds.contains_key(*key),
            "regression_thresholds must contain '{key}'"
        );
    }
}

#[test]
fn baseline_json_thresholds_consistent() {
    let path = workspace_root().join("benchmarks/baseline.json");
    assert!(path.exists(), "benchmarks/baseline.json must exist");

    let data: serde_json::Value =
        serde_json::from_reader(std::fs::File::open(&path).unwrap()).unwrap();

    let results = data["results"]
        .as_object()
        .expect("'results' must be an object");
    let thresholds = data["regression_thresholds"]
        .as_object()
        .expect("'regression_thresholds' must be an object");

    for key in results.keys() {
        assert!(
            thresholds.contains_key(key.as_str()),
            "baseline.json missing threshold for result key '{key}'"
        );
    }

    for (key, tval) in thresholds {
        let threshold = tval.as_f64().unwrap_or(0.0);
        let result = results
            .get(key.as_str())
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        assert!(
            threshold >= result,
            "baseline.json: threshold {threshold} < result {result} for '{key}'"
        );
    }
}

#[test]
fn update_benchmark_script_exists() {
    let path = workspace_root().join("scripts/update_benchmark_baseline.py");
    assert!(
        path.exists(),
        "scripts/update_benchmark_baseline.py must exist"
    );

    let content = std::fs::read_to_string(&path).unwrap();
    assert!(
        content.contains("--justification"),
        "script must require --justification argument"
    );
    assert!(
        content.contains("REGRESSION_ALLOWANCE"),
        "script must define REGRESSION_ALLOWANCE"
    );
    assert!(
        content.contains("baseline.json"),
        "script must write to baseline.json"
    );
}

#[test]
fn s3_express_validation_doc_present() {
    let path = workspace_root().join("docs/performance/s3-express-validation.md");
    assert!(
        path.exists(),
        "docs/performance/s3-express-validation.md must exist"
    );

    let content = std::fs::read_to_string(&path).unwrap();
    assert!(
        content.contains("acceptance gate"),
        "doc must describe the acceptance gate"
    );
    assert!(
        content.contains("ACCEPTED"),
        "doc must record the acceptance decision"
    );
}

#[test]
fn cost_analysis_doc_has_cost_per_operation_table() {
    let path = workspace_root().join("docs/performance/cost-analysis.md");
    assert!(
        path.exists(),
        "docs/performance/cost-analysis.md must exist"
    );

    let content = std::fs::read_to_string(&path).unwrap();
    assert!(
        content.contains("Cost-per-Operation"),
        "cost-analysis.md must have a Cost-per-Operation section"
    );
    assert!(
        content.contains("create_snapshot"),
        "cost-analysis.md must include create_snapshot in the cost table"
    );
    assert!(
        content.contains("0.023"),
        "cost-analysis.md must mention $0.023/GB-month S3 Standard price"
    );
}
