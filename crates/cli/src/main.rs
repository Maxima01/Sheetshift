use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::Serialize;
use sheetshift_core::{
    default_data_dir, supported_formats, ConversionRequest, HistoryEntry, Settings, Store,
    TERMS_VERSION,
};
use std::{io::Write, path::PathBuf, process::ExitCode};

const TERMS: &str = include_str!("../../../docs/TERMS.md");

/// Convert document text and spreadsheet data into Excel workbooks on your device.
#[derive(Debug, Parser)]
#[command(name = "sheetshift", version, about, arg_required_else_help = true)]
struct Cli {
    /// Directory containing sheetshift.sqlite3. Defaults to the shared desktop data directory.
    #[arg(long, global = true, value_name = "DIR")]
    data_dir: Option<PathBuf>,

    /// Accept the current application terms before executing the command.
    #[arg(long, global = true)]
    accept_terms: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Convert one or more supported files. Every input creates a separate .xlsx file.
    Convert(ConvertArgs),
    /// List conversion history or clear all saved history.
    History(HistoryArgs),
    /// View or update preferences shared with the desktop application.
    Settings {
        #[command(subcommand)]
        command: Option<SettingsCommand>,
    },
    /// Print the terms and conditions, or accept the current version.
    Terms {
        #[arg(long)]
        accept: bool,
    },
    /// List supported input extensions.
    Formats {
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Args)]
struct ConvertArgs {
    #[arg(required = true, value_name = "FILE", num_args = 1..)]
    files: Vec<PathBuf>,

    /// Output folder. Defaults to the saved preference, then ./converted.
    #[arg(short, long, value_name = "DIR")]
    output: Option<PathBuf>,

    /// Write a single JSON object to stdout; errors still cause a nonzero exit.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct HistoryArgs {
    #[arg(long, default_value_t = 50, value_parser = clap::value_parser!(u32).range(1..=1000))]
    limit: u32,
    #[arg(long)]
    json: bool,
    /// Delete all stored history entries, leaving files and settings intact.
    #[arg(long, requires = "yes")]
    clear: bool,
    /// Required acknowledgement for --clear.
    #[arg(long, requires = "clear")]
    yes: bool,
}

#[derive(Debug, Subcommand)]
enum SettingsCommand {
    /// Print the current settings as JSON.
    View,
    /// Set the default output folder, or remove the preference with --reset.
    OutputDir {
        #[arg(
            value_name = "DIR",
            required_unless_present = "reset",
            conflicts_with = "reset"
        )]
        directory: Option<PathBuf>,
        #[arg(long)]
        reset: bool,
    },
    /// Enable or disable recording future conversions. Existing entries are preserved.
    History { value: HistoryPreference },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum HistoryPreference {
    On,
    Off,
}

#[derive(Serialize)]
#[serde(untagged)]
enum ConversionOutcome {
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
struct BatchReport {
    succeeded: usize,
    failed: usize,
    results: Vec<ConversionOutcome>,
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::from(1)
        }
    }
}

fn run(cli: Cli) -> Result<bool> {
    // These read-only commands do not need to create a database.
    if !cli.accept_terms {
        match &cli.command {
            Command::Terms { accept: false } => {
                print_text(TERMS)?;
                return Ok(true);
            }
            Command::Formats { json } => {
                print_formats(*json)?;
                return Ok(true);
            }
            _ => {}
        }
    }

    let data_dir = cli.data_dir.map(Ok).unwrap_or_else(default_data_dir)?;
    std::fs::create_dir_all(&data_dir)
        .with_context(|| format!("cannot create data directory {}", data_dir.display()))?;
    let store = Store::open(data_dir.join("sheetshift.sqlite3"))?;
    if cli.accept_terms {
        store.accept_terms()?;
    }

    match cli.command {
        Command::Convert(args) => convert(&store, args),
        Command::History(args) => {
            if args.clear {
                store.clear_history()?;
                if args.json {
                    print_json(&serde_json::json!({ "cleared": true }))?;
                } else {
                    print_text("Conversion history cleared. Source and output files were kept.\n")?;
                }
            } else {
                let history = store.history(args.limit as usize)?;
                if args.json {
                    print_json(&history)?;
                } else if history.is_empty() {
                    print_text("No conversion history.\n")?;
                } else {
                    for entry in history {
                        print_text(&format!(
                            "{}  {}  {}\n",
                            entry.created_at, entry.status, entry.input_name
                        ))?;
                        if let Some(output) = entry.output_path {
                            print_text(&format!("  Output: {output}\n"))?;
                        }
                        if let Some(error) = entry.error {
                            print_text(&format!("  Error: {error}\n"))?;
                        }
                    }
                }
            }
            Ok(true)
        }
        Command::Settings { command } => {
            let mut settings = store.settings()?;
            match command {
                None | Some(SettingsCommand::View) => {}
                Some(SettingsCommand::OutputDir { directory, reset }) => {
                    settings.output_directory = if reset {
                        None
                    } else {
                        directory.map(absolute_path_string).transpose()?
                    };
                    store.save_settings(&settings)?;
                }
                Some(SettingsCommand::History { value }) => {
                    settings.remember_history = matches!(value, HistoryPreference::On);
                    store.save_settings(&settings)?;
                }
            }
            print_json(&settings)?;
            Ok(true)
        }
        Command::Terms { accept } => {
            if accept {
                store.accept_terms()?;
                print_text(&format!(
                    "Accepted SheetShift terms version {TERMS_VERSION}.\n"
                ))?;
            } else {
                print_text(TERMS)?;
            }
            Ok(true)
        }
        Command::Formats { json } => {
            print_formats(json)?;
            Ok(true)
        }
    }
}

fn convert(store: &Store, args: ConvertArgs) -> Result<bool> {
    let settings = store.settings()?;
    // Preparation failures become ordinary batch results, keeping --json useful
    // even when a destination is unwritable. Do not create files before consent.
    let output_directory = resolve_output_directory(args.output, &settings)
        .and_then(|directory| {
            if settings.accepted_terms_version.as_deref() == Some(TERMS_VERSION) {
                std::fs::create_dir_all(&directory).with_context(|| {
                    format!("cannot create output folder {directory}; choose a writable directory")
                })?;
            }
            Ok(directory)
        })
        .map_err(|error| format!("{error:#}"));
    let mut results = Vec::with_capacity(args.files.len());

    for file in args.files {
        let source_label = file.to_string_lossy().into_owned();
        let result = match &output_directory {
            Ok(output_directory) => absolute_path_string(file).and_then(|input_path| {
                store.convert(&ConversionRequest {
                    input_path,
                    output_directory: output_directory.clone(),
                })
            }),
            Err(error) => Err(anyhow::anyhow!(error.clone())),
        };
        let outcome = match result {
            Ok(entry) => ConversionOutcome::Stored(entry),
            Err(error) => ConversionOutcome::Rejected {
                input_path: source_label,
                status: "failed",
                error: format!("{error:#}"),
            },
        };
        if !args.json {
            print_outcome(&outcome)?;
        }
        results.push(outcome);
    }

    let succeeded = results.iter().filter(|entry| entry.is_success()).count();
    let report = BatchReport {
        succeeded,
        failed: results.len() - succeeded,
        results,
    };
    if args.json {
        print_json(&report)?;
    } else {
        print_text(&format!(
            "\n{} converted, {} failed.\n",
            report.succeeded, report.failed
        ))?;
    }
    Ok(report.failed == 0)
}

fn resolve_output_directory(explicit: Option<PathBuf>, settings: &Settings) -> Result<String> {
    let output = explicit
        .or_else(|| settings.output_directory.as_ref().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("converted"));
    absolute_path_string(output)
}

fn absolute_path_string(path: PathBuf) -> Result<String> {
    let absolute = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .context("cannot determine current working directory")?
            .join(path)
    };
    match absolute.to_str() {
        Some(path) => Ok(path.to_owned()),
        None => bail!("file paths must be valid Unicode: {}", absolute.display()),
    }
}

fn print_outcome(outcome: &ConversionOutcome) -> Result<()> {
    match outcome {
        ConversionOutcome::Stored(entry) => {
            if entry.status == "success" {
                print_text(&format!(
                    "Converted {} → {} ({} rows, {} sheets)\n",
                    entry.input_name,
                    entry.output_path.as_deref().unwrap_or("unknown output"),
                    entry.rows,
                    entry.sheets
                ))?;
            } else {
                print_text(&format!(
                    "Failed {}: {}\n",
                    entry.input_name,
                    entry.error.as_deref().unwrap_or("conversion failed")
                ))?;
            }
            for warning in &entry.warnings {
                print_text(&format!("  Warning: {warning}\n"))?;
            }
        }
        ConversionOutcome::Rejected {
            input_path, error, ..
        } => print_text(&format!("Failed {input_path}: {error}\n"))?,
    }
    Ok(())
}

fn print_formats(json: bool) -> Result<()> {
    let formats = supported_formats();
    if json {
        print_json(&formats)
    } else {
        print_text(&format!(
            "Supported input formats: {}\n",
            formats.join(", ")
        ))
    }
}

fn print_json<T: Serialize>(value: &T) -> Result<()> {
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    serde_json::to_writer_pretty(&mut output, value).context("cannot write JSON output")?;
    writeln!(output).context("cannot write output")?;
    Ok(())
}

fn print_text(value: &str) -> Result<()> {
    std::io::stdout()
        .lock()
        .write_all(value.as_bytes())
        .context("cannot write output")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clearing_history_requires_explicit_acknowledgement() {
        assert!(Cli::try_parse_from(["sheetshift", "history", "--clear"]).is_err());
        assert!(Cli::try_parse_from(["sheetshift", "history", "--yes"]).is_err());
        assert!(Cli::try_parse_from(["sheetshift", "history", "--clear", "--yes"]).is_ok());
    }

    #[test]
    fn conversion_accepts_multiple_files_and_global_flags_after_subcommand() {
        let cli = Cli::try_parse_from([
            "sheetshift",
            "convert",
            "letter.docx",
            "table.csv",
            "--output",
            "results",
            "--accept-terms",
            "--json",
            "--data-dir",
            "state",
        ])
        .unwrap();
        assert!(cli.accept_terms);
        assert_eq!(cli.data_dir, Some(PathBuf::from("state")));
        match cli.command {
            Command::Convert(args) => {
                assert_eq!(args.files.len(), 2);
                assert!(args.json);
                assert_eq!(args.output, Some(PathBuf::from("results")));
            }
            _ => panic!("expected convert command"),
        }
    }

    #[test]
    fn output_setting_requires_a_path_or_reset_and_disallows_both() {
        assert!(Cli::try_parse_from(["sheetshift", "settings", "output-dir"]).is_err());
        assert!(Cli::try_parse_from(["sheetshift", "settings", "output-dir", "--reset"]).is_ok());
        assert!(Cli::try_parse_from([
            "sheetshift",
            "settings",
            "output-dir",
            "results",
            "--reset",
        ])
        .is_err());
    }

    #[test]
    fn history_limit_is_bounded() {
        assert!(Cli::try_parse_from(["sheetshift", "history", "--limit", "0"]).is_err());
        assert!(Cli::try_parse_from(["sheetshift", "history", "--limit", "1001"]).is_err());
    }
}
