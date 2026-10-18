//! Batch conversion with interactive file picking, consent prompts,
//! animated progress, and JSON/plain reporting.

use crate::progress::BatchProgress;
use crate::prompts;
use crate::ui;
use anyhow::{Context, Result};
use serde::Serialize;
use sheetshift_core::{
    supported_formats, ConversionRequest, HistoryEntry, Settings, Store, TERMS_VERSION,
};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use super::absolute_path_string;
use super::info::TERMS;

/// Convert one or more supported files. Every input creates a separate .xlsx file.
#[derive(Debug, clap::Args)]
pub struct ConvertArgs {
    /// Files to convert. Omitted in a terminal opens an interactive file picker.
    #[arg(value_name = "FILE", num_args = 1..)]
    pub files: Vec<PathBuf>,

    /// Output folder. Defaults to the saved preference, then ./converted.
    #[arg(short, long, value_name = "DIR")]
    pub output: Option<PathBuf>,

    /// Write a single JSON object to stdout; errors still cause a nonzero exit.
    #[arg(long)]
    pub json: bool,
}

#[derive(Serialize)]
#[serde(untagged)]
pub enum ConversionOutcome {
    Stored(HistoryEntry),
    Rejected {
        input_path: String,
        status: &'static str,
        error: String,
    },
}

impl ConversionOutcome {
    fn is_success(&self) -> bool {
        matches!(self, Self::Stored(entry) if entry.status == "success")
    }
}

#[derive(Serialize)]
pub struct BatchReport {
    pub succeeded: usize,
    pub failed: usize,
    pub results: Vec<ConversionOutcome>,
}

/// Clap-style usage error for a non-interactive `convert` without files.
pub fn print_missing_files_error() {
    ui::eprint_line("error: the following required arguments were not provided:");
    ui::eprint_line("  <FILE>...");
    ui::eprint_line("");
    ui::eprint_line("Usage: sheetshift convert <FILE>...");
    ui::eprint_line("");
    ui::eprint_line("For more information, try '--help'.");
}

pub fn run(store: &Store, args: ConvertArgs, verbose: bool) -> Result<ExitCode> {
    let interactive = !args.json && ui::interactive();
    let mut files = args.files;

    if files.is_empty() {
        if !interactive {
            print_missing_files_error();
            return Ok(ExitCode::from(2));
        }
        files = prompts::pick_files(discover_candidates()?)?;
        if files.is_empty() {
            let cwd =
                std::env::current_dir().context("cannot determine current working directory")?;
            ui::eprint_line(&format!(
                "{} no supported files found in {}",
                ui::warn_label(),
                cwd.display()
            ));
            return Ok(ExitCode::FAILURE);
        }
        ui::verbose_log(
            verbose,
            format!("{} file(s) selected interactively", files.len()),
        );
    }

    let mut settings = store.settings()?;

    // Consent comes first: nothing on disk is created before acceptance.
    if interactive && settings.accepted_terms_version.as_deref() != Some(TERMS_VERSION) {
        if !prompts::terms_prompt(TERMS, TERMS_VERSION)? {
            ui::eprint_line(&format!(
                "{} terms not accepted; nothing was converted",
                ui::warn_label()
            ));
            return Ok(ExitCode::FAILURE);
        }
        store.accept_terms()?;
        settings = store.settings()?;
        ui::verbose_log(verbose, format!("accepted terms version {TERMS_VERSION}"));
    }

    // Destination precedence: --output, then an interactive prompt,
    // then the saved preference, then ./converted.
    let explicit_output = match args.output {
        Some(output) => Some(output),
        None if interactive => {
            let default = settings
                .output_directory
                .clone()
                .unwrap_or_else(|| "converted".to_owned());
            Some(PathBuf::from(prompts::input_line(
                "Output folder",
                &default,
                false,
            )?))
        }
        None => None,
    };

    // Preparation failures become ordinary batch results, keeping --json useful
    // even when a destination is unwritable. Do not create files before consent.
    let output_directory = resolve_output_directory(explicit_output, &settings)
        .and_then(|directory| {
            if settings.accepted_terms_version.as_deref() == Some(TERMS_VERSION) {
                std::fs::create_dir_all(&directory).with_context(|| {
                    format!("cannot create output folder {directory}; choose a writable directory")
                })?;
            }
            Ok(directory)
        })
        .map_err(|error| format!("{error:#}"));
    match &output_directory {
        Ok(directory) => ui::verbose_log(verbose, format!("output folder: {directory}")),
        Err(error) => ui::verbose_log(verbose, format!("output folder unusable: {error}")),
    }

    let total = files.len();
    let mut progress = BatchProgress::new(interactive, total);
    let mut results = Vec::with_capacity(total);

    for (index, file) in files.into_iter().enumerate() {
        let source_label = file.to_string_lossy().into_owned();
        let display_name = file
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| source_label.clone());
        progress.start_file(index, &display_name);

        let result = match &output_directory {
            Ok(output_directory) => absolute_path_string(file).and_then(|input_path| {
                ui::verbose_log(verbose, format!("converting {input_path}"));
                store.convert(&ConversionRequest {
                    input_path,
                    output_directory: output_directory.clone(),
                })
            }),
            Err(error) => Err(anyhow::anyhow!(error.clone())),
        };
        let outcome = match result {
            Ok(entry) => {
                ui::verbose_log(
                    verbose,
                    format!(
                        "{} finished in {}",
                        display_name,
                        ui::format_duration(entry.duration_ms)
                    ),
                );
                ConversionOutcome::Stored(entry)
            }
            Err(error) => ConversionOutcome::Rejected {
                input_path: source_label,
                status: "failed",
                error: format!("{error:#}"),
            },
        };

        let succeeded = outcome.is_success();
        progress.finish_file(&display_name, succeeded);
        if !interactive && !args.json {
            print_outcome(&outcome)?;
        }
        results.push(outcome);
    }
    progress.finish();

    let succeeded = results.iter().filter(|entry| entry.is_success()).count();
    let report = BatchReport {
        succeeded,
        failed: results.len() - succeeded,
        results,
    };
    if args.json {
        ui::print_json(&report)?;
    } else if interactive {
        print_interactive_summary(&report)?;
    } else {
        ui::print_text(&format!(
            "\n{} converted, {} failed.\n",
            report.succeeded, report.failed
        ))?;
    }
    Ok(if report.failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

/// Supported files in the current directory, sorted by name.
fn discover_candidates() -> Result<Vec<PathBuf>> {
    let cwd = std::env::current_dir().context("cannot determine current working directory")?;
    let supported = supported_formats();
    let mut files = Vec::new();
    for entry in
        std::fs::read_dir(&cwd).with_context(|| format!("cannot read {}", cwd.display()))?
    {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let path = entry.path();
        let extension = path
            .extension()
            .map(|value| value.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if supported.contains(&extension) {
            files.push(path);
        }
    }
    files.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    Ok(files)
}

fn resolve_output_directory(explicit: Option<PathBuf>, settings: &Settings) -> Result<String> {
    let output = explicit
        .or_else(|| settings.output_directory.as_ref().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("converted"));
    absolute_path_string(output)
}

fn print_outcome(outcome: &ConversionOutcome) -> Result<()> {
    match outcome {
        ConversionOutcome::Stored(entry) => {
            if entry.status == "success" {
                ui::print_text(&format!(
                    "Converted {} → {} ({} rows, {} sheets)\n",
                    entry.input_name,
                    entry.output_path.as_deref().unwrap_or("unknown output"),
                    entry.rows,
                    entry.sheets
                ))?;
            } else {
                ui::print_text(&format!(
                    "Failed {}: {}\n",
                    entry.input_name,
                    entry.error.as_deref().unwrap_or("conversion failed")
                ))?;
            }
            for warning in &entry.warnings {
                ui::print_text(&format!("  Warning: {warning}\n"))?;
            }
        }
        ConversionOutcome::Rejected {
            input_path, error, ..
        } => ui::print_text(&format!("Failed {input_path}: {error}\n"))?,
    }
    Ok(())
}

fn print_interactive_summary(report: &BatchReport) -> Result<()> {
    let rows: Vec<ui::ResultRow> = report
        .results
        .iter()
        .map(|outcome| match outcome {
            ConversionOutcome::Stored(entry) => ui::ResultRow {
                file: entry.input_name.clone(),
                status: entry.status.clone(),
                rows: entry.rows,
                sheets: entry.sheets,
                duration_ms: entry.duration_ms,
                detail: entry
                    .output_path
                    .clone()
                    .or_else(|| entry.error.clone())
                    .unwrap_or_default(),
            },
            ConversionOutcome::Rejected {
                input_path,
                status,
                error,
            } => ui::ResultRow {
                file: file_name(input_path),
                status: (*status).to_owned(),
                rows: 0,
                sheets: 0,
                duration_ms: 0,
                detail: error.clone(),
            },
        })
        .collect();

    let mut out = String::from("\n");
    out.push_str(&ui::section("Conversion summary"));
    out.push('\n');
    out.push_str(&ui::results_table(&rows));
    out.push('\n');

    let mut has_warnings = false;
    for outcome in &report.results {
        if let ConversionOutcome::Stored(entry) = outcome {
            for warning in &entry.warnings {
                if !has_warnings {
                    out.push('\n');
                    has_warnings = true;
                }
                out.push_str(&format!(
                    "  {} {}: {warning}\n",
                    ui::warning("warning:"),
                    entry.input_name
                ));
            }
        }
    }

    out.push('\n');
    if report.failed == 0 {
        out.push_str(&format!(
            "  {} {} converted\n",
            ui::success("✔"),
            report.succeeded
        ));
    } else {
        out.push_str(&format!(
            "  {} {} converted, {} failed\n",
            if report.succeeded == 0 {
                ui::failure("✖")
            } else {
                ui::warning("!")
            },
            report.succeeded,
            report.failed
        ));
    }
    ui::print_text(&out)
}

fn file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_owned())
}
