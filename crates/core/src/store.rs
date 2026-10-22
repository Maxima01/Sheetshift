use crate::{
    conversion, ConversionRequest, HistoryEntry, ReviewedConversionRequest, Settings, TERMS_VERSION,
};
use anyhow::{anyhow, bail, Context, Result};
use chrono::{SecondsFormat, Utc};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use std::{
    path::Path,
    sync::{Mutex, MutexGuard},
    time::{Duration, Instant},
};
use uuid::Uuid;

const HISTORY_LIMIT: usize = 1_000;

/// SQLite persistence. A mutex permits sharing the store between Tauri commands.
/// Parsing happens outside the database lock; separate CLI processes use SQLite's
/// busy timeout and WAL mode to coordinate concurrent reads and writes.
pub struct Store {
    connection: Mutex<Connection>,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Could not create data directory {}", parent.display()))?;
        }
        let connection = Connection::open(path)
            .with_context(|| format!("Could not open settings database {}", path.display()))?;
        connection.busy_timeout(Duration::from_secs(5))?;
        let version: u32 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version > 1 {
            bail!("This database uses schema version {version}, which requires a newer SheetShift version. Update the app or choose another database.");
        }
        connection
            .execute_batch(
                "PRAGMA journal_mode=WAL;
             PRAGMA secure_delete=ON;
             CREATE TABLE IF NOT EXISTS preferences (
                 id INTEGER PRIMARY KEY CHECK(id = 1),
                 value TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS conversion_history (
                 sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                 id TEXT NOT NULL UNIQUE,
                 value TEXT NOT NULL
             );
             PRAGMA user_version=1;",
            )
            .context("Could not initialize the settings database")?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>> {
        self.connection
            .lock()
            .map_err(|_| anyhow!("The settings database lock failed; restart SheetShift."))
    }

    pub fn settings(&self) -> Result<Settings> {
        let value: Option<String> = self
            .connection()?
            .query_row("SELECT value FROM preferences WHERE id=1", [], |row| {
                row.get(0)
            })
            .optional()?;
        value
            .map(|s| serde_json::from_str(&s).context("Stored preferences are invalid"))
            .unwrap_or_else(|| Ok(Settings::default()))
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let old_value: Option<String> = transaction
            .query_row("SELECT value FROM preferences WHERE id=1", [], |row| {
                row.get(0)
            })
            .optional()?;
        let current: Settings = old_value
            .map(|value| serde_json::from_str(&value))
            .transpose()?
            .unwrap_or_default();
        let mut settings = settings.clone();
        // A generic preferences update cannot accept or revoke legal consent.
        settings.accepted_terms_version = current.accepted_terms_version;
        transaction
            .execute(
                "INSERT INTO preferences(id, value) VALUES(1, ?1)
             ON CONFLICT(id) DO UPDATE SET value=excluded.value",
                [serde_json::to_string(&settings)?],
            )
            .context("Could not save preferences")?;
        transaction.commit()?;
        Ok(())
    }

    pub fn accept_terms(&self) -> Result<()> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let value: Option<String> = transaction
            .query_row("SELECT value FROM preferences WHERE id=1", [], |row| {
                row.get(0)
            })
            .optional()?;
        let mut settings: Settings = value
            .map(|value| serde_json::from_str(&value))
            .transpose()?
            .unwrap_or_default();
        settings.accepted_terms_version = Some(TERMS_VERSION.to_owned());
        transaction.execute(
            "INSERT INTO preferences(id, value) VALUES(1, ?1)
             ON CONFLICT(id) DO UPDATE SET value=excluded.value",
            [serde_json::to_string(&settings)?],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Latest first. Both retrieval and retention are capped at 1,000 attempts.
    pub fn history(&self, limit: usize) -> Result<Vec<HistoryEntry>> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare("SELECT value FROM conversion_history ORDER BY sequence DESC LIMIT ?1")?;
        let rows = statement.query_map([limit.min(HISTORY_LIMIT) as i64], |row| {
            row.get::<_, String>(0)
        })?;
        rows.map(|row| serde_json::from_str(&row?).context("A stored history entry is invalid"))
            .collect()
    }

    /// Removes metadata only; never deletes a source document or output workbook.
    pub fn clear_history(&self) -> Result<()> {
        let connection = self.connection()?;
        connection.execute("DELETE FROM conversion_history", [])?;
        // Reduce residual WAL content; filesystem backups can still retain old data.
        connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        Ok(())
    }

    /// Converts one document, recording success and conversion failures when enabled.
    /// Terms and persistence failures are returned as errors. File/parser failures
    /// return a failed entry so batch callers can continue with remaining files.
    pub fn convert(&self, request: &ConversionRequest) -> Result<HistoryEntry> {
        let settings = self.settings()?;
        if settings.accepted_terms_version.as_deref() != Some(TERMS_VERSION) {
            bail!("Please review and accept the current terms ({TERMS_VERSION}) before converting files.");
        }
        let start = Instant::now();
        let input = Path::new(&request.input_path);
        let absolute = input.canonicalize().unwrap_or_else(|_| input.to_path_buf());
        let mut entry = HistoryEntry {
            id: Uuid::new_v4().to_string(),
            input_name: input
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            input_path: absolute.to_string_lossy().into_owned(),
            output_path: None,
            format: input
                .extension()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase(),
            status: "failed".to_owned(),
            created_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            rows: 0,
            sheets: 0,
            duration_ms: 0,
            warnings: Vec::new(),
            error: None,
        };
        // Third-party parsers should return errors, but protect the application from
        // unwind panics caused by malformed input. This is not a resource sandbox.
        match std::panic::catch_unwind(|| conversion::convert(request)) {
            Ok(Ok(result)) => {
                entry.status = "success".to_owned();
                entry.output_path = Some(result.output_path.to_string_lossy().into_owned());
                entry.rows = result.rows;
                entry.sheets = result.sheets;
                entry.warnings = result.warnings;
            }
            Ok(Err(error)) => entry.error = Some(format!("{error:#}")),
            Err(_) => entry.error = Some("The document parser could not safely read this file. Re-export it from its original application and try again.".to_owned()),
        }
        entry.duration_ms = start.elapsed().as_millis().min(u64::MAX as u128) as u64;
        if settings.remember_history {
            self.record(&entry).with_context(|| {
                if let Some(output) = &entry.output_path {
                    format!(
                        "Workbook created at {output}, but its history record could not be saved"
                    )
                } else {
                    "Conversion failed and its history record could not be saved".to_owned()
                }
            })?;
        }
        Ok(entry)
    }

    /// Converts a document using the options and cell corrections confirmed in preview.
    pub fn convert_reviewed(&self, request: &ReviewedConversionRequest) -> Result<HistoryEntry> {
        let settings = self.settings()?;
        if settings.accepted_terms_version.as_deref() != Some(TERMS_VERSION) {
            bail!("Please review and accept the current terms ({TERMS_VERSION}) before converting files.");
        }
        let start = Instant::now();
        let input = Path::new(&request.input_path);
        let absolute = input.canonicalize().unwrap_or_else(|_| input.to_path_buf());
        let mut entry = HistoryEntry {
            id: Uuid::new_v4().to_string(),
            input_name: input
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            input_path: absolute.to_string_lossy().into_owned(),
            output_path: None,
            format: input
                .extension()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase(),
            status: "failed".to_owned(),
            created_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            rows: 0,
            sheets: 0,
            duration_ms: 0,
            warnings: Vec::new(),
            error: None,
        };
        match std::panic::catch_unwind(|| conversion::convert_reviewed(request)) {
            Ok(Ok(result)) => {
                entry.status = "success".to_owned();
                entry.output_path = Some(result.output_path.to_string_lossy().into_owned());
                entry.rows = result.rows;
                entry.sheets = result.sheets;
                entry.warnings = result.warnings;
            }
            Ok(Err(error)) => entry.error = Some(format!("{error:#}")),
            Err(_) => entry.error = Some("The document parser could not safely read this file. Re-export it from its original application and try again.".to_owned()),
        }
        entry.duration_ms = start.elapsed().as_millis().min(u64::MAX as u128) as u64;
        if settings.remember_history {
            self.record(&entry).with_context(|| {
                if let Some(output) = &entry.output_path {
                    format!(
                        "Workbook created at {output}, but its history record could not be saved"
                    )
                } else {
                    "Conversion failed and its history record could not be saved".to_owned()
                }
            })?;
        }
        Ok(entry)
    }

    fn record(&self, entry: &HistoryEntry) -> Result<()> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        // Check the current preference under the same write transaction as the
        // insertion so disabling history cannot race with a finishing conversion.
        let value: Option<String> = transaction
            .query_row("SELECT value FROM preferences WHERE id=1", [], |row| {
                row.get(0)
            })
            .optional()?;
        let settings: Settings = value
            .map(|value| serde_json::from_str(&value))
            .transpose()?
            .unwrap_or_default();
        if !settings.remember_history {
            return Ok(());
        }
        transaction.execute(
            "INSERT INTO conversion_history(id, value) VALUES(?1, ?2)",
            params![entry.id, serde_json::to_string(entry)?],
        )?;
        transaction.execute(
            "DELETE FROM conversion_history WHERE sequence NOT IN
             (SELECT sequence FROM conversion_history ORDER BY sequence DESC LIMIT ?1)",
            [HISTORY_LIMIT as i64],
        )?;
        transaction.commit()?;
        Ok(())
    }
}
