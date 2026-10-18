//! Preferences shared with the desktop application, with table output in
//! terminals and JSON everywhere else.

use super::absolute_path_string;
use crate::{prompts, ui};
use anyhow::Result;
use clap::ValueEnum;
use sheetshift_core::Store;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum HistoryPreference {
    On,
    Off,
}

#[derive(Debug, clap::Subcommand)]
pub enum SettingsCommand {
    /// Print the current settings.
    View {
        /// Write a single JSON object to stdout even in a terminal.
        #[arg(long)]
        json: bool,
    },
    /// Set the default output folder, or remove the preference with --reset.
    OutputDir {
        #[arg(value_name = "DIR", conflicts_with = "reset")]
        directory: Option<PathBuf>,
        #[arg(long)]
        reset: bool,
    },
    /// Enable or disable recording future conversions. Existing entries are preserved.
    History { value: Option<HistoryPreference> },
}

pub fn run(store: &Store, command: Option<SettingsCommand>, verbose: bool) -> Result<ExitCode> {
    let interactive = ui::interactive();
    let view_json = matches!(&command, Some(SettingsCommand::View { json: true }));
    let mut settings = store.settings()?;
    let mut note: Option<String> = None;

    match command {
        None | Some(SettingsCommand::View { .. }) => {}
        Some(SettingsCommand::OutputDir { directory, reset }) => {
            if reset {
                settings.output_directory = None;
                note = Some(format!(
                    "{} output folder preference removed",
                    ui::success("✔")
                ));
                ui::verbose_log(verbose, "output folder preference removed");
            } else if let Some(directory) = directory {
                let absolute = absolute_path_string(directory)?;
                settings.output_directory = Some(absolute.clone());
                note = Some(format!(
                    "{} output folder set to {}",
                    ui::success("✔"),
                    ui::accent(&absolute)
                ));
                ui::verbose_log(verbose, format!("output folder set to {absolute}"));
            } else if interactive {
                let current = settings.output_directory.clone().unwrap_or_default();
                let chosen =
                    prompts::input_line("Output folder (leave empty to reset)", &current, true)?;
                let chosen = chosen.trim().to_owned();
                settings.output_directory = if chosen.is_empty() {
                    note = Some(format!(
                        "{} output folder preference removed",
                        ui::success("✔")
                    ));
                    None
                } else {
                    let absolute = absolute_path_string(PathBuf::from(&chosen))?;
                    note = Some(format!(
                        "{} output folder set to {}",
                        ui::success("✔"),
                        ui::accent(&absolute)
                    ));
                    Some(absolute)
                };
                ui::verbose_log(
                    verbose,
                    match &settings.output_directory {
                        Some(directory) => format!("output folder set to {directory}"),
                        None => "output folder preference removed".to_owned(),
                    },
                );
            } else {
                ui::eprint_line("error: settings output-dir requires <DIR> or --reset");
                return Ok(ExitCode::from(2));
            }
            store.save_settings(&settings)?;
        }
        Some(SettingsCommand::History { value }) => {
            let enabled = match value {
                Some(HistoryPreference::On) => true,
                Some(HistoryPreference::Off) => false,
                None if interactive => prompts::history_preference(settings.remember_history)?,
                None => {
                    ui::eprint_line("error: settings history requires 'on' or 'off'");
                    return Ok(ExitCode::from(2));
                }
            };
            settings.remember_history = enabled;
            store.save_settings(&settings)?;
            note = Some(if enabled {
                format!("{} history recording enabled", ui::success("✔"))
            } else {
                format!("{} history recording disabled", ui::warning("✔"))
            });
            ui::verbose_log(verbose, format!("remember history: {enabled}"));
        }
    }

    if view_json || !interactive {
        ui::print_json(&settings)?;
    } else {
        let mut out = String::new();
        if let Some(note) = &note {
            out.push_str(&format!("  {note}\n\n"));
        }
        out.push_str(&ui::section("Settings"));
        out.push('\n');
        out.push_str(&ui::settings_table(&settings));
        out.push('\n');
        ui::print_text(&out)?;
    }
    Ok(ExitCode::SUCCESS)
}
