//! Read-only information commands: terms and supported formats.

use crate::ui;
use anyhow::Result;
use sheetshift_core::{supported_formats, Store, TERMS_VERSION};
use std::process::ExitCode;

/// The versioned terms shown by both interfaces.
pub const TERMS: &str = include_str!("../../../../docs/TERMS.md");

#[derive(Debug, clap::Args)]
pub struct TermsArgs {
    /// Accept the current terms without printing them.
    #[arg(long)]
    pub accept: bool,
}

#[derive(Debug, clap::Args)]
pub struct FormatsArgs {
    /// Write the format list as JSON.
    #[arg(long)]
    pub json: bool,
}

/// Print the terms. Read-only: never opens the database.
pub fn print_terms() -> Result<ExitCode> {
    let mut out = ui::section(format!("SheetShift terms and conditions ({TERMS_VERSION})"));
    out.push('\n');
    out.push_str(TERMS);
    ui::print_text(&out)?;
    Ok(ExitCode::SUCCESS)
}

/// Record acceptance of the current terms version.
pub fn accept_terms(store: &Store) -> Result<ExitCode> {
    store.accept_terms()?;
    ui::print_text(&format!(
        "Accepted SheetShift terms version {}.\n",
        ui::success(TERMS_VERSION)
    ))?;
    Ok(ExitCode::SUCCESS)
}

/// List supported input extensions: JSON, a table, or a plain sentence.
pub fn formats(args: &FormatsArgs) -> Result<ExitCode> {
    let formats = supported_formats();
    if args.json {
        ui::print_json(&formats)?;
        return Ok(ExitCode::SUCCESS);
    }
    if ui::interactive() {
        let rows: Vec<(String, String)> = formats
            .iter()
            .map(|extension| (extension.clone(), description(extension)))
            .collect();
        let mut out = ui::section("Supported input formats");
        out.push('\n');
        out.push_str(&ui::formats_table(&rows));
        out.push('\n');
        ui::print_text(&out)?;
        return Ok(ExitCode::SUCCESS);
    }
    ui::print_text(&format!(
        "Supported input formats: {}\n",
        formats.join(", ")
    ))?;
    Ok(ExitCode::SUCCESS)
}

fn description(extension: &str) -> String {
    match extension {
        "docx" => "Word document paragraphs and tables",
        "pdf" => "PDF text lines (no OCR)",
        "xlsx" | "xls" | "xlsb" | "xlsm" => "Excel workbook values",
        "ods" => "OpenDocument spreadsheet values",
        "csv" => "Comma-separated text rows",
        "tsv" => "Tab-separated text rows",
        "txt" => "Plain text lines",
        "json" => "JSON data as rows and columns",
        _ => "Supported input",
    }
    .to_owned()
}
