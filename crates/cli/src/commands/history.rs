//! History listing (styled table or plain lines), JSON output, and clearing.

use crate::{prompts, ui};
use anyhow::Result;
use sheetshift_core::Store;
use std::process::ExitCode;

#[derive(Debug, clap::Args)]
pub struct HistoryArgs {
    #[arg(long, default_value_t = 50, value_parser = clap::value_parser!(u32).range(1..=1000))]
    pub limit: u32,

    #[arg(long)]
    pub json: bool,

    /// Delete all stored history entries, leaving files and settings intact.
    #[arg(long)]
    pub clear: bool,

    /// Required acknowledgement for --clear outside interactive sessions.
    #[arg(long)]
    pub yes: bool,
}

pub fn run(store: &Store, args: HistoryArgs, verbose: bool) -> Result<ExitCode> {
    let interactive = !args.json && ui::interactive();

    if args.yes && !args.clear {
        ui::eprint_line("error: --yes requires --clear");
        return Ok(ExitCode::from(2));
    }

    if args.clear {
        return clear(store, &args, interactive, verbose);
    }
    list(store, &args, interactive)
}

fn clear(store: &Store, args: &HistoryArgs, interactive: bool, verbose: bool) -> Result<ExitCode> {
    if !args.yes {
        if interactive {
            let count = store.history(1_000)?.len();
            let question =
                format!("Delete all {count} history entries? Documents and workbooks are kept.");
            if !prompts::confirm(&question, false)? {
                return Err(ui::Cancelled.into());
            }
        } else {
            ui::eprint_line("error: --clear requires --yes");
            return Ok(ExitCode::from(2));
        }
    }

    store.clear_history()?;
    ui::verbose_log(verbose, "history cleared");
    if args.json {
        ui::print_json(&serde_json::json!({ "cleared": true }))?;
    } else if interactive {
        ui::print_text(&format!(
            "  {}\n",
            ui::success("history cleared — documents and workbooks were kept")
        ))?;
    } else {
        ui::print_text("Conversion history cleared. Source and output files were kept.\n")?;
    }
    Ok(ExitCode::SUCCESS)
}

fn list(store: &Store, args: &HistoryArgs, interactive: bool) -> Result<ExitCode> {
    let history = store.history(args.limit as usize)?;
    if args.json {
        ui::print_json(&history)?;
        return Ok(ExitCode::SUCCESS);
    }
    if history.is_empty() {
        if interactive {
            ui::print_text(&format!(
                "{}\n  {}\n",
                ui::section("Conversion history"),
                ui::dim("no conversions recorded yet")
            ))?;
        } else {
            ui::print_text("No conversion history.\n")?;
        }
        return Ok(ExitCode::SUCCESS);
    }
    if !interactive {
        for entry in history {
            ui::print_text(&format!(
                "{}  {}  {}\n",
                entry.created_at, entry.status, entry.input_name
            ))?;
            if let Some(output) = entry.output_path {
                ui::print_text(&format!("  Output: {output}\n"))?;
            }
            if let Some(error) = entry.error {
                ui::print_text(&format!("  Error: {error}\n"))?;
            }
        }
        return Ok(ExitCode::SUCCESS);
    }

    let mut out = ui::section(format!("Conversion history ({} entries)", history.len()));
    out.push('\n');
    out.push_str(&ui::history_table(&history));
    out.push('\n');
    let mut has_warnings = false;
    for entry in &history {
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
    ui::print_text(&out)?;
    Ok(ExitCode::SUCCESS)
}
