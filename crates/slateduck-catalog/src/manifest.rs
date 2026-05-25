//! Manifest log for catalog state management.
//!
//! Provides a write-ahead manifest that tracks active SST files, compaction
//! operations, and writer epochs for fencing.

/// A manifest log tracking SST lifecycle and writer epochs.
#[derive(Debug, Clone, Default)]
pub struct ManifestLog {
    entries: Vec<ManifestEntry>,
    current_epoch: u64,
    pending_compaction: bool,
    active_ssts: Vec<String>,
}

/// A single manifest entry.
#[derive(Debug, Clone)]
pub struct ManifestEntry {
    pub kind: ManifestEntryKind,
    pub sst_id: Option<String>,
}

/// Kind of manifest entry.
#[derive(Debug, Clone)]
pub enum ManifestEntryKind {
    AddSst,
    RemoveSst,
    BeginCompaction,
    AbortCompaction,
}

impl ManifestEntry {
    /// Create an entry for a new SST file.
    pub fn new_sst(sst_id: &str) -> Self {
        Self {
            kind: ManifestEntryKind::AddSst,
            sst_id: Some(sst_id.to_string()),
        }
    }

    /// Get the SST ID if this entry references one.
    pub fn sst_id(&self) -> Option<&str> {
        self.sst_id.as_deref()
    }
}

impl ManifestLog {
    /// Create a new empty manifest log.
    pub fn new() -> Self {
        Self::default()
    }

    /// Get all entries.
    pub fn entries(&self) -> &[ManifestEntry] {
        &self.entries
    }

    /// Get active SST file IDs.
    pub fn active_ssts(&self) -> Vec<String> {
        self.active_ssts.clone()
    }

    /// Begin a compaction operation.
    pub fn begin_compaction(&mut self, input_ssts: &[String], _output_sst: &str) {
        self.pending_compaction = true;
        // Add inputs as active if not already present.
        for sst in input_ssts {
            if !self.active_ssts.contains(sst) {
                self.active_ssts.push(sst.clone());
            }
        }
    }

    /// Check if there's a pending compaction.
    pub fn has_pending_compaction(&self) -> bool {
        self.pending_compaction
    }

    /// Abort a pending compaction (discard output, keep inputs).
    pub fn abort_compaction(&mut self) {
        self.pending_compaction = false;
    }

    /// Acquire a new writer epoch.
    pub fn acquire_epoch(&mut self) -> u64 {
        self.current_epoch += 1;
        self.current_epoch
    }

    /// Try to write an entry with a given epoch. Fails if epoch is stale (fenced).
    pub fn try_write(&mut self, epoch: u64, entry: ManifestEntry) -> Result<(), String> {
        if epoch < self.current_epoch {
            return Err(format!(
                "fenced: writer epoch {} < current epoch {}",
                epoch, self.current_epoch
            ));
        }
        if let Some(sst_id) = &entry.sst_id {
            self.active_ssts.push(sst_id.clone());
        }
        self.entries.push(entry);
        Ok(())
    }
}
