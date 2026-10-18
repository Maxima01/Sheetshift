use anyhow::{Context, Result};
use clap::{CommandFactory, Parser, Subcommand};
use sheetshift_core::{default_data_dir, Store};
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

mod commands;
mod progress;
mod prompts;
mod ui;

use commands::{ConvertArgs, FormatsArgs, HistoryArgs, SettingsCommand, TermsArgs};

const EXIT_USAGE: u8 = 2;
const EXIT_INTERRUPTED: u8 = 130;

const EXAMPLES: &str = "\
Examples:
  sheetshift                              Open the interactive menu
  sheetshift convert                      Pick files with fuzzy search
  sheetshift convert a.docx -o ./books    Convert with animated progress
  sheetshift history                      Styled history table
  sheetshift history --json               Machine-readable history
";

/// Convert document text and spreadsheet data into Excel workbooks on your device.
#[derive(Debug, Parser)]
#[command(name = "sheetshift", version, about, after_help = EXAMPLES)]
struct Cli {
    /// Directory containing sheetshift.sqlite3. Defaults to the shared desktop data directory.
    #[arg(long, global = true, value_name = "DIR")]
    data_dir: Option<PathBuf>,

    /// Accept the current application terms before executing the command.
    #[arg(long, global = true)]
    accept_terms: bool,

    /// Print extra diagnostics on stderr.
    #[arg(short, long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Option<Command>,
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
    Terms(TermsArgs),
    /// List supported input extensions.
    Formats(FormatsArgs),
}

/// Set by the Ctrl-C handler so interrupted prompts can wait for its notice.
static INTERRUPTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn main() -> ExitCode {
    install_interrupt_handler();
    match run(Cli::parse()) {
        Ok(code) => code,
        Err(error) => {
            if error.downcast_ref::<ui::Cancelled>().is_some() {
                ui::eprint_line(&ui::cancelled_note());
                ExitCode::from(EXIT_INTERRUPTED)
            } else if is_interrupted_read(&error) {
                // Let the Ctrl-C handler print the cancellation notice before
                // this thread tears the process down.
                let deadline = std::time::Instant::now() + std::time::Duration::from_millis(300);
                while !INTERRUPTED.load(std::sync::atomic::Ordering::SeqCst)
                    && std::time::Instant::now() < deadline
                {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                ExitCode::from(EXIT_INTERRUPTED)
            } else {
                ui::eprint_line(&format!("{} {:#}", ui::error_label(), error));
                ExitCode::from(1)
            }
        }
    }
}

/// True when an interactive read was aborted by Ctrl-C.
fn is_interrupted_read(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        if let Some(io) = cause.downcast_ref::<std::io::Error>() {
            io.kind() == std::io::ErrorKind::Interrupted
        } else if let Some(dialoguer::Error::IO(io)) = cause.downcast_ref::<dialoguer::Error>() {
            io.kind() == std::io::ErrorKind::Interrupted
        } else {
            false
        }
    })
}

fn run(cli: Cli) -> Result<ExitCode> {
    let Cli {
        data_dir,
        accept_terms,
        verbose,
        command,
    } = cli;

    // These read-only commands do not need to create a database.
    if !accept_terms {
        match &command {
            Some(Command::Terms(args)) if !args.accept => return commands::info::print_terms(),
            Some(Command::Formats(args)) => return commands::info::formats(args),
            _ => {}
        }
    }

    // A bare invocation opens the interactive menu on a terminal and prints
    // help with a usage exit code everywhere else.
    let Some(command) = command else {
        if ui::interactive() {
            return menu_flow(data_dir, verbose);
        }
        print_help_to_stderr();
        return Ok(ExitCode::from(EXIT_USAGE));
    };

    // Fail usage checks before creating any local state.
    if let Command::Convert(args) = &command {
        if args.files.is_empty() && (args.json || !ui::interactive()) {
            commands::convert::print_missing_files_error();
            return Ok(ExitCode::from(EXIT_USAGE));
        }
    }

    let store = open_store(data_dir, verbose)?;
    if accept_terms {
        store.accept_terms()?;
    }

    match command {
        Command::Convert(args) => commands::convert::run(&store, args, verbose),
        Command::History(args) => commands::history::run(&store, args, verbose),
        Command::Settings { command } => commands::settings::run(&store, command, verbose),
        Command::Terms(args) if args.accept => commands::info::accept_terms(&store),
        Command::Terms(_) => commands::info::print_terms(),
        Command::Formats(args) => commands::info::formats(&args),
    }
}

fn open_store(data_dir: Option<PathBuf>, verbose: bool) -> Result<Store> {
    let data_dir = data_dir.map(Ok).unwrap_or_else(default_data_dir)?;
    ui::verbose_log(verbose, format!("data directory: {}", data_dir.display()));
    std::fs::create_dir_all(&data_dir)
        .with_context(|| format!("cannot create data directory {}", data_dir.display()))?;
    Store::open(data_dir.join("sheetshift.sqlite3"))
}

/// Main menu flow for the bare `sheetshift` invocation on a terminal.
fn menu_flow(data_dir: Option<PathBuf>, verbose: bool) -> Result<ExitCode> {
    ui::print_text(&ui::banner())?;
    match prompts::main_menu()? {
        prompts::MainMenu::Convert => {
            let store = open_store(data_dir, verbose)?;
            commands::convert::run(
                &store,
                ConvertArgs {
                    files: Vec::new(),
                    output: None,
                    json: false,
                },
                verbose,
            )
        }
        prompts::MainMenu::History => {
            let store = open_store(data_dir, verbose)?;
            commands::history::run(
                &store,
                HistoryArgs {
                    limit: 50,
                    json: false,
                    clear: false,
                    yes: false,
                },
                verbose,
            )
        }
        prompts::MainMenu::Settings => {
            let store = open_store(data_dir, verbose)?;
            commands::settings::run(&store, None, verbose)
        }
        prompts::MainMenu::Terms => commands::info::print_terms(),
        prompts::MainMenu::Formats => commands::info::formats(&FormatsArgs { json: false }),
        prompts::MainMenu::Exit => Ok(ExitCode::SUCCESS),
    }
}

fn print_help_to_stderr() {
    let help = Cli::command().render_help();
    let mut stderr = std::io::stderr().lock();
    let _ = writeln!(stderr, "{help}");
    let _ = stderr.flush();
}

/// Restore the cursor and leave cleanly on Ctrl-C instead of a stack trace.
fn install_interrupt_handler() {
    let _ = ctrlc::set_handler(|| {
        INTERRUPTED.store(true, std::sync::atomic::Ordering::SeqCst);
        let term = console::Term::stderr();
        let _ = term.show_cursor();
        let _ = term.write_line("");
        let _ = term.write_line(&ui::cancelled_note());
        std::process::exit(i32::from(EXIT_INTERRUPTED));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_invocation_parses_without_a_subcommand() {
        let cli = Cli::try_parse_from(["sheetshift"]).unwrap();
        assert!(cli.command.is_none());
        assert!(!cli.verbose);
    }

    #[test]
    fn clearing_history_flags_parse_and_are_validated_at_runtime() {
        assert!(Cli::try_parse_from(["sheetshift", "history", "--clear"]).is_ok());
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
            Some(Command::Convert(args)) => {
                assert_eq!(args.files.len(), 2);
                assert!(args.json);
                assert_eq!(args.output, Some(PathBuf::from("results")));
            }
            _ => panic!("expected convert command"),
        }
    }

    #[test]
    fn output_setting_disallows_a_path_together_with_reset() {
        assert!(Cli::try_parse_from(["sheetshift", "settings", "output-dir"]).is_ok());
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

    #[test]
    fn verbose_flag_is_global() {
        assert!(Cli::try_parse_from(["sheetshift", "-v", "formats"]).is_ok());
        assert!(Cli::try_parse_from(["sheetshift", "formats", "--verbose"]).is_ok());
        assert!(Cli::try_parse_from(["sheetshift", "history", "-v"]).is_ok());
    }
}
