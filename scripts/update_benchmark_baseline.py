#!/usr/bin/env python3
"""Update the stored benchmark regression baseline.

Usage:
    python scripts/update_benchmark_baseline.py \\
        --source benchmarks/v0.42-catalog-bench.json \\
        --justification "v0.42.0: new TPC-H catalog bench with 16-reader concurrency"

The script reads `measurements` from the source file, computes 110% thresholds
(10% regression allowance), and writes `benchmarks/baseline.json`.

A justification comment is required so that intentional baseline updates are
auditable in git history.

Exit codes:
    0 — baseline updated successfully
    1 — argument error or file not found
    2 — source file has no 'measurements' key
"""

import argparse
import json
import sys
from datetime import timezone, datetime
from pathlib import Path


BASELINE_PATH = Path("benchmarks/baseline.json")
REGRESSION_ALLOWANCE = 1.10  # 10% regression budget


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Update benchmarks/baseline.json from a new measurement file."
    )
    parser.add_argument(
        "--source",
        required=True,
        help="Path to the source benchmark JSON (e.g. benchmarks/v0.42-catalog-bench.json)",
    )
    parser.add_argument(
        "--justification",
        required=True,
        help="One-line reason for the baseline update (required for auditability)",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Print what would be written without modifying baseline.json",
    )
    return parser.parse_args()


def load_json(path: Path) -> dict:
    if not path.exists():
        print(f"ERROR: File not found: {path}", file=sys.stderr)
        sys.exit(1)
    with open(path) as f:
        return json.load(f)


def main() -> None:
    args = parse_args()
    source_path = Path(args.source)
    source = load_json(source_path)

    measurements = source.get("measurements")
    if not measurements:
        print(
            f"ERROR: source file {source_path} has no 'measurements' key",
            file=sys.stderr,
        )
        sys.exit(2)

    # Extract p50_us as the representative metric for each operation.
    results: dict[str, float] = {}
    for key, value in measurements.items():
        if isinstance(value, dict):
            # Use p50_us if available; fall back to the first numeric value.
            if "p50_us" in value:
                results[key] = value["p50_us"]
            else:
                for v in value.values():
                    if isinstance(v, (int, float)):
                        results[key] = v
                        break
        elif isinstance(value, (int, float)):
            results[key] = value

    thresholds = {k: round(v * REGRESSION_ALLOWANCE) for k, v in results.items()}

    baseline = {
        "benchmark": "v0.42-benchmark-regression-baseline",
        "version": source.get("version", "unknown"),
        "timestamp": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "note": (
            f"Updated by update_benchmark_baseline.py. "
            f"Justification: {args.justification}. "
            f"Source: {source_path}. "
            f"Thresholds are {int((REGRESSION_ALLOWANCE - 1) * 100)}% above p50 measurements."
        ),
        "storage": source.get("storage", "LocalFileSystem (tmpdir)"),
        "results": results,
        "regression_thresholds": thresholds,
    }

    if args.dry_run:
        print("DRY RUN — would write the following to", BASELINE_PATH)
        print(json.dumps(baseline, indent=2))
        return

    with open(BASELINE_PATH, "w") as f:
        json.dump(baseline, f, indent=2)
        f.write("\n")

    print(f"Updated {BASELINE_PATH} with {len(results)} metrics.")
    print(f"Justification: {args.justification}")
    print(
        "\nRemember to commit benchmarks/baseline.json with a message that "
        "includes the justification."
    )


if __name__ == "__main__":
    main()
