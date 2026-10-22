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
    /// Copy an unchanged .xlsx package so styles, merges, formulas, and other
    /// workbook features stay intact. Other input formats are unsupported.
    #[serde(default)]
    pub preserve_formatting: bool,
}

/// How PDF content should be prepared for review.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PdfReviewMode {
    /// Reconstruct content on the requested pages using the same table parser.
    /// An empty page selection means every page.
    SelectedPages,
    /// Reconstruct ruled tables from positioned text and cell borders, with a
    /// spacing-based fallback for simple text-layer tables.
    #[default]
    DetectTables,
}

/// PDF-specific options shared by preview and reviewed conversion.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct PdfReviewOptions {
    pub mode: PdfReviewMode,
    /// One-based pages and inclusive ranges, for example `1-3, 7, 10-12`.
    pub page_selection: Option<String>,
}

/// A request to inspect a source before writing a workbook.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PreviewRequest {
    pub input_path: String,
    #[serde(default)]
    pub pdf_options: PdfReviewOptions,
}

/// A JSON-friendly cell used by the editable preview.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum PreviewCell {
    Text(String),
    Number(f64),
    Boolean(bool),
    Empty,
}

/// A bounded view of one eventual workbook sheet.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PreviewSheet {
    pub name: String,
    pub rows: Vec<Vec<PreviewCell>>,
    pub total_rows: usize,
    pub total_columns: usize,
    pub truncated: bool,
}

/// Prepared source data shown to the user before conversion.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConversionPreview {
    pub input_name: String,
    pub format: String,
    /// SHA-256 of the reviewed source bytes, checked again before export.
    pub source_fingerprint: String,
    pub page_count: Option<u32>,
    /// Pages included in PDF extraction. In table mode this includes text-only
    /// pages whose paragraphs are kept alongside detected tables.
    pub selected_pages: Vec<u32>,
    pub detected_tables: usize,
    pub sheets: Vec<PreviewSheet>,
    pub warnings: Vec<String>,
}

/// A single cell correction made in the preview grid.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CellEdit {
    pub sheet_index: usize,
    pub row_index: usize,
    pub column_index: usize,
    pub value: String,
}

/// A conversion that reuses review options and applies explicit user corrections.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewedConversionRequest {
    pub input_path: String,
    pub output_directory: String,
    pub source_fingerprint: String,
    #[serde(default)]
    pub pdf_options: PdfReviewOptions,
    #[serde(default)]
    pub edits: Vec<CellEdit>,
    #[serde(default)]
    pub sheet_names: Vec<String>,
    #[serde(default)]
    pub preserve_formatting: bool,
}

/// Prepares a bounded, editable preview without creating output or history.
pub fn preview_file(request: &PreviewRequest) -> Result<ConversionPreview> {
    conversion::preview(request)
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
