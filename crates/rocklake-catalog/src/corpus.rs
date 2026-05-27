//! Wire corpus diff and validation tools.
//!
//! `corpus diff` compares two wire-corpus fixture files and emits a structured
//! diff of all statement families, handshake probes, and type OID requests that
//! changed between versions.
//!
//! `corpus validate` replays a corpus fixture file against the current dispatcher
//! and reports which statement families are handled, which need dispatcher updates
//! (category-b), and which require new SQL operator types (category-c).

#![allow(missing_docs)]

use std::collections::{HashMap, HashSet};
use std::io::BufRead;

use serde::{Deserialize, Serialize};

// ─── Corpus Record ─────────────────────────────────────────────────────────

/// A single entry in a wire-corpus NDJSON fixture file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorpusRecord {
    /// Statement family, e.g. "SELECT", "INSERT", "SET", "SHOW", "BEGIN", etc.
    #[serde(default)]
    pub statement_family: String,
    /// The raw SQL text (may be templated with $N parameters).
    #[serde(default)]
    pub sql: String,
    /// Table name targeted, if applicable.
    #[serde(default)]
    pub table: String,
    /// Whether this is a handshake probe (startup sequence).
    #[serde(default)]
    pub is_handshake: bool,
    /// PostgreSQL type OIDs requested in this exchange.
    #[serde(default)]
    pub type_oids: Vec<u32>,
    /// Protocol type: "simple" or "extended".
    #[serde(default)]
    pub protocol: String,
    /// Optional tag for classifier result.
    #[serde(default)]
    pub category: String,
}

// ─── Diff ──────────────────────────────────────────────────────────────────

/// A single entry in the corpus diff.
#[derive(Debug, Clone)]
pub struct DiffEntry {
    pub change_type: DiffChangeType,
    pub statement_family: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffChangeType {
    Added,
    Removed,
    Modified,
}

impl std::fmt::Display for DiffChangeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Added => write!(f, "ADDED"),
            Self::Removed => write!(f, "REMOVED"),
            Self::Modified => write!(f, "MODIFIED"),
        }
    }
}

/// Compare two corpus files and return a list of differences.
pub fn corpus_diff(old_records: &[CorpusRecord], new_records: &[CorpusRecord]) -> Vec<DiffEntry> {
    let mut old_families: HashMap<String, Vec<&CorpusRecord>> = HashMap::new();
    let mut new_families: HashMap<String, Vec<&CorpusRecord>> = HashMap::new();

    for r in old_records {
        old_families
            .entry(r.statement_family.clone())
            .or_default()
            .push(r);
    }
    for r in new_records {
        new_families
            .entry(r.statement_family.clone())
            .or_default()
            .push(r);
    }

    let all_families: HashSet<&String> = old_families.keys().chain(new_families.keys()).collect();
    let mut diffs = Vec::new();

    for family in all_families {
        match (old_families.get(family), new_families.get(family)) {
            (None, Some(new)) => {
                diffs.push(DiffEntry {
                    change_type: DiffChangeType::Added,
                    statement_family: family.clone(),
                    detail: format!("{} new statement(s)", new.len()),
                });
            }
            (Some(old), None) => {
                diffs.push(DiffEntry {
                    change_type: DiffChangeType::Removed,
                    statement_family: family.clone(),
                    detail: format!("{} statement(s) removed", old.len()),
                });
            }
            (Some(old), Some(new)) => {
                if old.len() != new.len() {
                    diffs.push(DiffEntry {
                        change_type: DiffChangeType::Modified,
                        statement_family: family.clone(),
                        detail: format!("count {} → {}", old.len(), new.len()),
                    });
                }
                // Check for type OID changes
                let old_oids: HashSet<u32> = old
                    .iter()
                    .flat_map(|r| r.type_oids.iter().copied())
                    .collect();
                let new_oids: HashSet<u32> = new
                    .iter()
                    .flat_map(|r| r.type_oids.iter().copied())
                    .collect();
                let added_oids: Vec<u32> = new_oids.difference(&old_oids).copied().collect();
                let removed_oids: Vec<u32> = old_oids.difference(&new_oids).copied().collect();
                if !added_oids.is_empty() || !removed_oids.is_empty() {
                    let detail = format!("type OIDs changed: +{added_oids:?} -{removed_oids:?}");
                    diffs.push(DiffEntry {
                        change_type: DiffChangeType::Modified,
                        statement_family: family.clone(),
                        detail,
                    });
                }
            }
            _ => {}
        }
    }

    diffs.sort_by_key(|d| d.statement_family.clone());
    diffs
}

// ─── Validate ─────────────────────────────────────────────────────────────

/// Result of validating a corpus against the current dispatcher.
#[derive(Debug, Clone, Default)]
pub struct ValidateResult {
    /// Statement families that are fully supported today.
    pub category_a: Vec<String>,
    /// Statement families that need minor dispatcher extensions (category-b).
    pub category_b: Vec<String>,
    /// Statement families that require new SQL operator types (category-c).
    pub category_c: Vec<String>,
    /// Handshake probes: always category-a.
    pub handshake_count: usize,
}

impl ValidateResult {
    pub fn print(&self) {
        println!("Corpus Validation");
        println!("=================");
        println!(
            "  Handshake probes:  {} (all supported)",
            self.handshake_count
        );
        println!();
        println!("Category A — Fully supported ({}):", self.category_a.len());
        for s in &self.category_a {
            println!("  ✓ {s}");
        }
        if !self.category_b.is_empty() {
            println!();
            println!(
                "Category B — Minor dispatcher extension needed ({}):",
                self.category_b.len()
            );
            for s in &self.category_b {
                println!("  ~ {s}");
            }
        }
        if !self.category_c.is_empty() {
            println!();
            println!(
                "Category C — New SQL operator type required ({}):",
                self.category_c.len()
            );
            for s in &self.category_c {
                println!("  ✗ {s}");
            }
        }
    }
}

/// Known-supported statement families in the current dispatcher.
const CATEGORY_A_FAMILIES: &[&str] = &[
    "SELECT",
    "INSERT",
    "UPDATE",
    "BEGIN",
    "COMMIT",
    "ROLLBACK",
    "SET",
    "SHOW",
    "CREATE TABLE",
    "DROP TABLE",
];

/// Families that need minor extensions but are within the bounded SQL set.
const CATEGORY_B_FAMILIES: &[&str] = &[
    "SELECT WITH ORDER BY",
    "SELECT WITH LIMIT",
    "INSERT WITH gen_random_uuid",
    "UPDATE TABLE STATS",
];

/// Validate a set of corpus records against the current dispatcher.
pub fn corpus_validate(records: &[CorpusRecord]) -> ValidateResult {
    let mut result = ValidateResult::default();
    let mut seen_families: HashSet<String> = HashSet::new();

    for r in records {
        if r.is_handshake {
            result.handshake_count += 1;
            continue;
        }
        if r.statement_family.is_empty() {
            continue;
        }
        if seen_families.contains(&r.statement_family) {
            continue;
        }
        seen_families.insert(r.statement_family.clone());

        if CATEGORY_A_FAMILIES.contains(&r.statement_family.as_str()) {
            result.category_a.push(r.statement_family.clone());
        } else if CATEGORY_B_FAMILIES.contains(&r.statement_family.as_str()) {
            result.category_b.push(r.statement_family.clone());
        } else {
            result.category_c.push(r.statement_family.clone());
        }
    }

    result
}

// ─── File Parsing ──────────────────────────────────────────────────────────

/// Parse a corpus NDJSON file from a reader.
/// Lines that fail to parse are skipped with a warning.
pub fn parse_corpus<R: BufRead>(reader: R) -> Vec<CorpusRecord> {
    let mut records = Vec::new();
    for (lineno, line) in reader.lines().enumerate() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!("corpus: line {} read error: {e}", lineno + 1);
                break;
            }
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match serde_json::from_str::<CorpusRecord>(trimmed) {
            Ok(r) => records.push(r),
            Err(e) => {
                tracing::warn!("corpus: line {} parse error: {e}", lineno + 1);
            }
        }
    }
    records
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn make_record(family: &str) -> CorpusRecord {
        CorpusRecord {
            statement_family: family.to_string(),
            sql: String::new(),
            table: String::new(),
            is_handshake: false,
            type_oids: vec![],
            protocol: "simple".to_string(),
            category: String::new(),
        }
    }

    #[test]
    fn corpus_diff_detects_added_family() {
        let old = vec![make_record("SELECT")];
        let new = vec![make_record("SELECT"), make_record("INSERT")];
        let diff = corpus_diff(&old, &new);
        assert!(diff
            .iter()
            .any(|d| d.statement_family == "INSERT" && d.change_type == DiffChangeType::Added));
    }

    #[test]
    fn corpus_diff_detects_removed_family() {
        let old = vec![make_record("SELECT"), make_record("UPDATE")];
        let new = vec![make_record("SELECT")];
        let diff = corpus_diff(&old, &new);
        assert!(diff
            .iter()
            .any(|d| d.statement_family == "UPDATE" && d.change_type == DiffChangeType::Removed));
    }

    #[test]
    fn corpus_validate_classifies_categories() {
        let records = vec![
            make_record("SELECT"),
            make_record("INSERT"),
            make_record("SOME_EXOTIC_DDL"),
        ];
        let result = corpus_validate(&records);
        assert!(result.category_a.contains(&"SELECT".to_string()));
        assert!(result.category_c.contains(&"SOME_EXOTIC_DDL".to_string()));
    }

    #[test]
    fn parse_corpus_skips_bad_lines() {
        let data =
            "{\"statement_family\":\"SELECT\"}\nnot json\n{\"statement_family\":\"INSERT\"}\n";
        let cursor = Cursor::new(data);
        let records = parse_corpus(cursor);
        assert_eq!(records.len(), 2);
    }
}
