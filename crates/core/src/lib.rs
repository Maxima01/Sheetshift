//! The shared, offline conversion engine used by the desktop application and CLI.
//!
//! [`Store`] persists only preferences and conversion metadata. Documents are read
//! from the supplied local paths and generated XLSX files stay in the chosen folder.

mod conversion;
mod store;

use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub use store::Store;

/// A change to the terms requires explicit acceptance again.
pub const TERMS_VERSION: &str = "2026-10-15";

/// Preferences shared by the desktop UI and command line application.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Settings {
    pub output_directory: Option<String>,
    pub remember_history: bool,
    pub accepted_terms_version: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            output_directory: None,
            remember_history: true,
            accepted_terms_version: None,
        }
    }
}

/// A single conversion. Paths may be absolute or relative to the caller's CWD.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversionRequest {
    pub input_path: String,
    pub output_directory: String,
}

/// An immutable record of an attempted conversion.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryEntry {
    pub id: String,
    pub input_name: String,
    pub input_path: String,
    pub output_path: Option<String>,
    pub format: String,
    /// Either `success` or `failed`.
    pub status: String,
    /// RFC 3339 timestamp in UTC.
    pub created_at: String,
    pub rows: u64,
    pub sheets: u32,
    pub duration_ms: u64,
    pub warnings: Vec<String>,
    pub error: Option<String>,
}

/// The per-user application data directory. The caller chooses the database name.
pub fn default_data_dir() -> Result<PathBuf> {
    let dirs = ProjectDirs::from("com", "sheetshift", "SheetShift")
        .context("Could not locate your user data directory; provide a custom database path.")?;
    Ok(dirs.data_local_dir().to_path_buf())
}

/// Supported file extensions, without a leading dot.
pub fn supported_formats() -> Vec<String> {
    [
        "docx", "pdf", "xlsx", "xls", "xlsb", "xlsm", "ods", "csv", "tsv", "txt", "json",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect()
}
