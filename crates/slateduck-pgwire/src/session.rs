//! Session state for PG-Wire connections.
//!
//! Tracks transaction state, pending operations, and session settings.

use slateduck_sql::CatalogOp;

/// Maximum pending batch size (64 MiB).
pub const MAX_PENDING_BATCH_SIZE: usize = 64 * 1024 * 1024;

/// Session state for a single PG connection.
pub struct Session {
    /// Whether we're in a transaction block.
    pub in_transaction: bool,
    /// Pending catalog operations buffered between BEGIN and COMMIT.
    pub pending_ops: Vec<CatalogOp>,
    /// Estimated size of pending operations.
    pub pending_size: usize,
    /// Session settings.
    pub settings: SessionSettings,
}

/// Session settings (SET/SHOW).
pub struct SessionSettings {
    pub timezone: String,
    pub client_encoding: String,
    pub date_style: String,
    pub server_version: String,
    pub transaction_isolation: String,
}

impl Default for SessionSettings {
    fn default() -> Self {
        Self {
            timezone: "UTC".to_string(),
            client_encoding: "UTF8".to_string(),
            date_style: "ISO, YMD".to_string(),
            server_version: "16.0".to_string(),
            transaction_isolation: "serializable".to_string(),
        }
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    pub fn new() -> Self {
        Self {
            in_transaction: false,
            pending_ops: Vec::new(),
            pending_size: 0,
            settings: SessionSettings::default(),
        }
    }

    /// Begin a transaction.
    pub fn begin(&mut self) {
        self.in_transaction = true;
        self.pending_ops.clear();
        self.pending_size = 0;
    }

    /// Buffer an operation for commit.
    pub fn buffer_op(&mut self, op: CatalogOp) -> Result<(), BatchTooLarge> {
        // Rough size estimate
        let op_size = std::mem::size_of_val(&op) + 256;
        if self.pending_size + op_size > MAX_PENDING_BATCH_SIZE {
            return Err(BatchTooLarge);
        }
        self.pending_size += op_size;
        self.pending_ops.push(op);
        Ok(())
    }

    /// Commit: drain pending ops and reset.
    pub fn commit(&mut self) -> Vec<CatalogOp> {
        self.in_transaction = false;
        self.pending_size = 0;
        std::mem::take(&mut self.pending_ops)
    }

    /// Rollback: discard pending ops.
    pub fn rollback(&mut self) {
        self.in_transaction = false;
        self.pending_ops.clear();
        self.pending_size = 0;
    }

    /// Get the value of a session setting.
    pub fn get_setting(&self, name: &str) -> String {
        match name.to_lowercase().as_str() {
            "timezone" | "time zone" => self.settings.timezone.clone(),
            "client_encoding" => self.settings.client_encoding.clone(),
            "datestyle" => self.settings.date_style.clone(),
            "server_version" => self.settings.server_version.clone(),
            "transaction_isolation" | "default_transaction_isolation" => {
                self.settings.transaction_isolation.clone()
            }
            "standard_conforming_strings" => "on".to_string(),
            "integer_datetimes" => "on".to_string(),
            "server_encoding" => "UTF8".to_string(),
            "is_superuser" => "on".to_string(),
            "session_authorization" => "slateduck".to_string(),
            _ => String::new(),
        }
    }

    /// Set a session setting.
    pub fn set_setting(&mut self, name: &str, value: &str) {
        match name.to_lowercase().as_str() {
            "timezone" | "time zone" => self.settings.timezone = value.to_string(),
            "client_encoding" => self.settings.client_encoding = value.to_string(),
            "datestyle" => self.settings.date_style = value.to_string(),
            _ => {} // Accept and ignore other settings
        }
    }
}

/// Error when the pending transaction batch exceeds 64 MiB.
#[derive(Debug)]
pub struct BatchTooLarge;

impl std::fmt::Display for BatchTooLarge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "pending batch exceeds 64 MiB limit")
    }
}

impl std::error::Error for BatchTooLarge {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_transaction_lifecycle() {
        let mut session = Session::new();
        assert!(!session.in_transaction);

        session.begin();
        assert!(session.in_transaction);

        session.buffer_op(CatalogOp::Begin).unwrap();
        assert_eq!(session.pending_ops.len(), 1);

        let ops = session.commit();
        assert!(!session.in_transaction);
        assert_eq!(ops.len(), 1);
        assert!(session.pending_ops.is_empty());
    }

    #[test]
    fn session_rollback_clears_ops() {
        let mut session = Session::new();
        session.begin();
        session.buffer_op(CatalogOp::Commit).unwrap();
        session.rollback();
        assert!(!session.in_transaction);
        assert!(session.pending_ops.is_empty());
    }

    #[test]
    fn session_settings() {
        let mut session = Session::new();
        assert_eq!(session.get_setting("timezone"), "UTC");
        session.set_setting("timezone", "America/New_York");
        assert_eq!(session.get_setting("timezone"), "America/New_York");
        assert_eq!(session.get_setting("server_version"), "16.0");
    }
}
