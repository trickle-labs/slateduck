#!/usr/bin/env bash
# ducklake-compat.sh — Optional DuckLake compatibility corpus runner.
#
# Runs each SQL statement in the corpus fixtures against a local DuckDB binary
# that has the `ducklake` extension installed and reports new failures as
# actionable diffs against the expected (empty-error) baseline.
#
# Usage:
#   bash scripts/ducklake-compat.sh
#   make ducklake-compat       # if a Makefile target exists
#
# Requirements:
#   - DuckDB CLI installed (duckdb >= 1.2)
#   - ducklake extension loadable by DuckDB (LOAD ducklake)
#   - SLATEDUCK_COMPAT_FIXTURES env var pointing to corpus dir
#     (default: tests/fixtures/corpus/)
#
# Environment variables:
#   DUCKDB_BIN           path to duckdb binary (default: duckdb)
#   SLATEDUCK_COMPAT_FIXTURES  path to corpus fixtures dir
#   SLATEDUCK_COMPAT_FAIL_FAST  set to 1 to stop on first failure (default: 0)
#
# Exit codes:
#   0  all corpus statements executed without error
#   1  one or more statements failed (diffs printed to stdout)
#   2  duckdb binary not found or ducklake extension unavailable

set -euo pipefail

DUCKDB_BIN="${DUCKDB_BIN:-duckdb}"
FIXTURES_DIR="${SLATEDUCK_COMPAT_FIXTURES:-tests/fixtures/corpus}"
FAIL_FAST="${SLATEDUCK_COMPAT_FAIL_FAST:-0}"

# ── Preflight ──────────────────────────────────────────────────────────────────

if ! command -v "$DUCKDB_BIN" >/dev/null 2>&1; then
    echo "ERROR: duckdb binary not found at '$DUCKDB_BIN'." >&2
    echo "Install DuckDB >= 1.2 or set DUCKDB_BIN to the correct path." >&2
    exit 2
fi

DUCKDB_VERSION=$("$DUCKDB_BIN" --version 2>&1 | head -1)
echo "DuckDB: $DUCKDB_VERSION"

# Check ducklake extension is available.
if ! "$DUCKDB_BIN" -c "LOAD ducklake;" >/dev/null 2>&1; then
    echo "WARNING: ducklake extension could not be loaded. Skipping corpus run." >&2
    echo "Install the ducklake extension: INSTALL ducklake; or use DuckDB >= 1.2 with bundled ducklake." >&2
    exit 2
fi

echo "ducklake extension: available"

# ── Corpus discovery ───────────────────────────────────────────────────────────

if [[ ! -d "$FIXTURES_DIR" ]]; then
    echo "ERROR: corpus fixtures directory '$FIXTURES_DIR' not found." >&2
    echo "Capture corpus first by running: cargo test -p slateduck-pgwire corpus" >&2
    exit 2
fi

mapfile -t CORPUS_FILES < <(find "$FIXTURES_DIR" -name "*.sql" | sort)
if [[ ${#CORPUS_FILES[@]} -eq 0 ]]; then
    echo "INFO: No corpus .sql files found in '$FIXTURES_DIR'. Nothing to run."
    exit 0
fi

echo "Running ${#CORPUS_FILES[@]} corpus file(s) against DuckDB $DUCKDB_VERSION…"

# ── Run each corpus file ───────────────────────────────────────────────────────

FAILURES=0
TMPDB=$(mktemp /tmp/slateduck-compat-XXXXXX.db)
trap 'rm -f "$TMPDB"' EXIT

for corpus_file in "${CORPUS_FILES[@]}"; do
    echo -n "  $(basename "$corpus_file") … "
    error_output=$("$DUCKDB_BIN" "$TMPDB" < "$corpus_file" 2>&1) || true
    # Remove expected no-op output (DuckDB echoes "Run Time" and similar).
    actual_errors=$(echo "$error_output" | grep -v "^Run Time" | grep -iE "Error|Exception|panic" || true)
    if [[ -n "$actual_errors" ]]; then
        echo "FAIL"
        echo "    $actual_errors"
        FAILURES=$((FAILURES + 1))
        if [[ "$FAIL_FAST" == "1" ]]; then
            echo "FAIL_FAST=1: stopping after first failure."
            exit 1
        fi
    else
        echo "ok"
    fi
    # Reset the DB between files for isolation.
    rm -f "$TMPDB"
done

# ── Summary ────────────────────────────────────────────────────────────────────

echo ""
if [[ $FAILURES -gt 0 ]]; then
    echo "RESULT: $FAILURES corpus file(s) produced unexpected errors."
    echo "Review the diffs above and update the corpus or fix the executor."
    exit 1
else
    echo "RESULT: all corpus files passed."
    exit 0
fi
