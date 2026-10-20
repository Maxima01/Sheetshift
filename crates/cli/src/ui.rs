//! Visual theme, terminal capability detection, and structured output helpers.
//!
//! Rich output (menus, spinners, tables) is reserved for interactive terminals.
//! Piped sessions, `TERM=dumb`, `NO_COLOR`, and CI fall back to plain text.

use anyhow::{Context, Result};
use console::style;
use serde::Serialize;
use sheetshift_core::{HistoryEntry, Settings};
use std::ffi::OsStr;
use std::io::{IsTerminal, Write};
use tabled::{
    settings::{object::Columns, object::Rows, Alignment, Color, Modify, Style, Width},
    Table, Tabled,
};

/// Returned when the user dismisses a prompt with Esc or `q`.
#[derive(Debug)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("cancelled")
    }
}

impl std::error::Error for Cancelled {}

/// True when menus, spinners, and colored tables should be used.
pub fn interactive() -> bool {
    !ci_environment()
        && !dumb_terminal()
        && std::io::stdin().is_terminal()
        && std::io::stderr().is_terminal()
}

fn ci_environment() -> bool {
    matches!(std::env::var("CI"), Ok(value) if !matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "" | "0" | "false" | "no"
    ))
}

fn dumb_terminal() -> bool {
    std::env::var_os("TERM").as_deref() == Some(OsStr::new("dumb"))
}

// ---------------------------------------------------------------- palette ---

pub fn accent(text: impl std::fmt::Display) -> String {
    style(text).cyan().bold().to_string()
}

pub fn success(text: impl std::fmt::Display) -> String {
    style(text).green().bold().to_string()
}

pub fn failure(text: impl std::fmt::Display) -> String {
    style(text).red().bold().to_string()
}

pub fn warning(text: impl std::fmt::Display) -> String {
    style(text).yellow().to_string()
}

pub fn dim(text: impl std::fmt::Display) -> String {
    style(text).dim().to_string()
}

pub fn error_label() -> String {
    style("error:").red().bold().for_stderr().to_string()
}

pub fn warn_label() -> String {
    style("warning:").yellow().bold().for_stderr().to_string()
}

pub fn cancelled_note() -> String {
    style("Cancelled.").yellow().bold().for_stderr().to_string()
}

/// A compact colored chip for a conversion status.
pub fn status_badge(status: &str) -> String {
    match status {
        "success" => style(" success ").on_green().black().bold().to_string(),
        _ => style(" failed ").on_red().white().bold().to_string(),
    }
}

/// Styled banner shown when the interactive main menu starts.
pub fn banner() -> String {
    format!(
        "{} {}\n{}\n",
        style("◼ SheetShift").cyan().bold(),
        style(format!("v{}", env!("CARGO_PKG_VERSION"))).dim(),
        style("local document → Excel conversion").dim(),
    )
}

/// Styled section heading followed by a dim rule.
pub fn section(title: impl std::fmt::Display) -> String {
    format!("{}\n{}", style(title).bold().cyan(), dim("─".repeat(24)))
}

/// Diagnostics for `-v/--verbose`, written to stderr only.
pub fn verbose_log(enabled: bool, message: impl std::fmt::Display) {
    if !enabled {
        return;
    }
    let line = format!(
        "{} {}",
        style("·").dim().for_stderr(),
        style(message).dim().for_stderr()
    );
    let _ = writeln!(std::io::stderr(), "{line}");
}

// ---------------------------------------------------------------- output ---

pub fn print_text(value: &str) -> Result<()> {
    std::io::stdout()
        .lock()
        .write_all(value.as_bytes())
        .context("cannot write output")
}

pub fn print_json<T: Serialize>(value: &T) -> Result<()> {
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    serde_json::to_writer_pretty(&mut output, value).context("cannot write JSON output")?;
    writeln!(output).context("cannot write output")?;
    Ok(())
}

pub fn eprint_line(value: &str) {
    let _ = writeln!(std::io::stderr(), "{value}");
}

// --------------------------------------------------------------- formats ---

/// `12:34:56`-style elapsed formatting for durations in milliseconds.
pub fn format_duration(ms: u64) -> String {
    if ms < 1_000 {
        format!("{ms}ms")
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1_000.0)
    } else {
        format!("{}m {:02}s", ms / 60_000, (ms % 60_000) / 1_000)
    }
}

/// `2026-10-18 14:23:01.123Z` → `2026-10-18 14:23:01`.
pub fn compact_timestamp(timestamp: &str) -> String {
    let (date, time) = timestamp.split_once('T').unwrap_or((timestamp, ""));
    let time = time.split('.').next().unwrap_or(time);
    let time = time.trim_end_matches('Z');
    format!("{date} {time}")
}

fn terminal_cols() -> usize {
    let (_, cols) = console::Term::stdout().size();
    let cols = if cols == 0 { 100 } else { usize::from(cols) };
    cols.clamp(60, 160)
}

/// Splits the width left over for two variable-length columns.
fn split_two(available: usize) -> (usize, usize) {
    let first = (available * 2 / 5).max(8);
    let second = available.saturating_sub(first).max(8);
    (first, second)
}

// ---------------------------------------------------------------- tables ---

fn style_table(table: &mut Table) {
    table.with(Style::rounded());
    if console::colors_enabled() {
        let header = Color::new("\u{1b}[1m\u{1b}[96m", "\u{1b}[0m");
        table.with(Modify::new(Rows::new(0..1)).with(header));
    }
}

/// History records as a bordered table with status badges and truncated paths.
pub fn history_table(entries: &[HistoryEntry]) -> String {
    #[derive(Tabled)]
    struct Row {
        #[tabled(rename = "WHEN")]
        when: String,
        #[tabled(rename = "STATUS")]
        status: String,
        #[tabled(rename = "FILE")]
        file: String,
        #[tabled(rename = "ROWS")]
        rows: String,
        #[tabled(rename = "DETAIL")]
        detail: String,
    }

    let rows = entries.iter().map(|entry| Row {
        when: compact_timestamp(&entry.created_at),
        status: status_badge(&entry.status),
        file: entry.input_name.clone(),
        rows: if entry.status == "success" {
            entry.rows.to_string()
        } else {
            "—".to_owned()
        },
        detail: detail_text(entry),
    });

    // WHEN(19) + STATUS(9) + ROWS(6) plus borders, padding, and two open columns.
    let available = terminal_cols().saturating_sub(16 + 34);
    let (file_width, detail_width) = split_two(available);

    let mut table = Table::new(rows);
    table.with(Modify::new(Columns::one(2)).with(Width::truncate(file_width)));
    table.with(Modify::new(Columns::one(4)).with(Width::truncate(detail_width)));
    table.with(Modify::new(Columns::one(3)).with(Alignment::right()));
    style_table(&mut table);
    table.to_string()
}

fn detail_text(entry: &HistoryEntry) -> String {
    if let Some(error) = &entry.error {
        return error.clone();
    }
    if let Some(output) = &entry.output_path {
        return output.clone();
    }
    if let Some(warning) = entry.warnings.first() {
        return format!("warning: {warning}");
    }
    String::new()
}

/// One conversion result row for the post-batch summary table.
pub struct ResultRow {
    pub file: String,
    pub status: String,
    pub rows: u64,
    pub sheets: u32,
    pub duration_ms: u64,
    pub detail: String,
}

/// Summary table for a finished conversion batch.
pub fn results_table(rows: &[ResultRow]) -> String {
    #[derive(Tabled)]
    struct Row {
        #[tabled(rename = "FILE")]
        file: String,
        #[tabled(rename = "STATUS")]
        status: String,
        #[tabled(rename = "ROWS")]
        rows: String,
        #[tabled(rename = "SHEETS")]
        sheets: String,
        #[tabled(rename = "TIME")]
        time: String,
        #[tabled(rename = "DETAIL")]
        detail: String,
    }

    let records = rows.iter().map(|row| Row {
        file: row.file.clone(),
        status: status_badge(&row.status),
        rows: if row.status == "success" {
            row.rows.to_string()
        } else {
            "—".to_owned()
        },
        sheets: if row.status == "success" {
            row.sheets.to_string()
        } else {
            "—".to_owned()
        },
        time: format_duration(row.duration_ms),
        detail: row.detail.clone(),
    });

    // STATUS(9) + ROWS(6) + SHEETS(6) + TIME(7) plus borders, padding, open columns.
    let available = terminal_cols().saturating_sub(19 + 28);
    let (file_width, detail_width) = split_two(available);

    let mut table = Table::new(records);
    table.with(Modify::new(Columns::one(0)).with(Width::truncate(file_width)));
    table.with(Modify::new(Columns::one(5)).with(Width::truncate(detail_width)));
    table.with(Modify::new(Columns::one(2)).with(Alignment::right()));
    table.with(Modify::new(Columns::one(3)).with(Alignment::right()));
    style_table(&mut table);
    table.to_string()
}

/// Preferences as a two-column key/value table.
pub fn settings_table(settings: &Settings) -> String {
    #[derive(Tabled)]
    struct Row {
        #[tabled(rename = "SETTING")]
        setting: String,
        #[tabled(rename = "VALUE")]
        value: String,
    }

    let output_directory = match &settings.output_directory {
        Some(directory) => accent(directory.clone()),
        None => dim("(not set)"),
    };
    let history = if settings.remember_history {
        success("on")
    } else {
        dim("off")
    };
    let terms = match &settings.accepted_terms_version {
        Some(version) => success(version.clone()),
        None => warning("not accepted"),
    };

    let rows = vec![
        Row {
            setting: "Output folder".to_owned(),
            value: output_directory,
        },
        Row {
            setting: "Remember history".to_owned(),
            value: history,
        },
        Row {
            setting: "Accepted terms".to_owned(),
            value: terms,
        },
    ];

    let mut table = Table::new(rows);
    table.with(
        Modify::new(Columns::one(1)).with(Width::truncate(terminal_cols().saturating_sub(7 + 18))),
    );
    style_table(&mut table);
    table.to_string()
}

/// Supported input formats with short descriptions.
pub fn formats_table(formats: &[(String, String)]) -> String {
    #[derive(Tabled)]
    struct Row {
        #[tabled(rename = "EXTENSION")]
        extension: String,
        #[tabled(rename = "DESCRIPTION")]
        description: String,
    }

    let rows = formats.iter().map(|(extension, description)| Row {
        extension: accent(format!(".{extension}")),
        description: description.clone(),
    });

    let mut table = Table::new(rows);
    style_table(&mut table);
    table.to_string()
}
