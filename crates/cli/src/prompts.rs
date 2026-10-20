//! Interactive prompts: main menu, fuzzy multi-select file picking,
//! confirmations, and input dialogs built on dialoguer and console.

use crate::ui;
use anyhow::{Context, Result};
use console::{Key, Term};
use dialoguer::{theme::ColorfulTheme, Confirm, Input, Select};
use fuzzy_matcher::{skim::SkimMatcherV2, FuzzyMatcher};
use std::path::PathBuf;

/// Top-level choices for the bare `sheetshift` invocation.
pub enum MainMenu {
    Convert,
    History,
    Settings,
    Terms,
    Formats,
    Exit,
}

pub fn main_menu() -> Result<MainMenu> {
    let items = [
        "Convert files to Excel",
        "View conversion history",
        "Settings & preferences",
        "Terms & conditions",
        "Supported formats",
        "Exit",
    ];
    let choice = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Choose an action  (↑/↓ move · Enter select · Esc quit)")
        .items(items)
        .default(0)
        .interact_opt()
        .context("the interactive menu could not start")?;
    Ok(match choice {
        Some(0) => MainMenu::Convert,
        Some(1) => MainMenu::History,
        Some(2) => MainMenu::Settings,
        Some(3) => MainMenu::Terms,
        Some(4) => MainMenu::Formats,
        _ => MainMenu::Exit,
    })
}

/// Yes/no prompt. Esc or `q` cancels the whole command.
pub fn confirm(prompt: &str, default: bool) -> Result<bool> {
    Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt(prompt)
        .default(default)
        .interact_opt()
        .context("the confirmation prompt could not start")?
        .ok_or_else(|| ui::Cancelled.into())
}

/// Single-line text prompt with a default value.
pub fn input_line(prompt: &str, default: &str, permit_empty: bool) -> Result<String> {
    Input::with_theme(&ColorfulTheme::default())
        .with_prompt(prompt)
        .default(default.to_owned())
        .allow_empty(permit_empty)
        .interact()
        .context("the input prompt could not start")
}

/// Show a terms preview on stderr and ask for consent.
pub fn terms_prompt(terms: &str, version: &str) -> Result<bool> {
    let term = Term::stderr();
    term.write_line(&ui::section(format!("Terms of use ({version})")))?;
    for line in terms.lines().take(8) {
        term.write_line(line)?;
    }
    term.write_line(&ui::dim("… run `sheetshift terms` to read the full text"))?;
    confirm("Do you accept these terms?", false)
}

/// Radio list for the history recording preference. Returns `true` for on.
pub fn history_preference(current: bool) -> Result<bool> {
    let items = ["on", "off"];
    let choice = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Record future conversions in history?")
        .items(items)
        .default(if current { 0 } else { 1 })
        .interact_opt()
        .context("the history preference prompt could not start")?;
    match choice {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(ui::Cancelled.into()),
    }
}

/// Fuzzy multi-select over candidate files: type to filter, arrows to move,
/// space to toggle, Enter to confirm, Esc to cancel.
pub fn pick_files(candidates: Vec<PathBuf>) -> Result<Vec<PathBuf>> {
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    FilePicker::new(candidates).run()
}

struct FilePicker {
    labels: Vec<String>,
    checked: Vec<bool>,
    paths: Vec<PathBuf>,
    query: String,
    cursor: usize,
}

impl FilePicker {
    fn new(paths: Vec<PathBuf>) -> Self {
        let labels: Vec<String> = paths
            .iter()
            .map(|path| {
                path.file_name()
                    .unwrap_or(path.as_os_str())
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        let checked = vec![false; labels.len()];
        Self {
            labels,
            checked,
            paths,
            query: String::new(),
            cursor: 0,
        }
    }

    /// Candidate indices matching the query, best score first.
    fn filtered(&self) -> Vec<usize> {
        if self.query.is_empty() {
            return (0..self.labels.len()).collect();
        }
        let matcher = SkimMatcherV2::default();
        let mut scored: Vec<(usize, i64)> = self
            .labels
            .iter()
            .enumerate()
            .filter_map(|(index, label)| {
                matcher.fuzzy_match(label, &self.query).map(|s| (index, s))
            })
            .collect();
        scored.sort_by_key(|&(_, score)| std::cmp::Reverse(score));
        scored.into_iter().map(|(index, _)| index).collect()
    }

    fn run(mut self) -> Result<Vec<PathBuf>> {
        let term = Term::stderr();
        term.hide_cursor()?;
        let outcome = self.event_loop(&term);
        let _ = term.show_cursor();
        outcome
    }

    fn event_loop(&mut self, term: &Term) -> Result<Vec<PathBuf>> {
        let mut drawn = 0usize;
        let result: Result<()> = loop {
            if drawn > 0 {
                term.clear_last_lines(drawn)?;
            }
            let filtered = self.filtered();
            if self.cursor >= filtered.len() {
                self.cursor = filtered.len().saturating_sub(1);
            }
            let (rows, cols) = term.size();
            let cols = usize::from(cols).max(40);
            let max_visible = (usize::from(rows).saturating_sub(4)).clamp(1, 12);

            let lines = self.render(&filtered, max_visible, cols);
            for line in &lines {
                term.write_line(line)?;
            }
            drawn = lines.len();
            term.flush()?;

            match term.read_key()? {
                Key::ArrowUp => self.cursor = self.cursor.saturating_sub(1),
                Key::ArrowDown => {
                    self.cursor = (self.cursor + 1).min(filtered.len().saturating_sub(1))
                }
                Key::Home => self.cursor = 0,
                Key::End => self.cursor = filtered.len().saturating_sub(1),
                Key::Char(' ') if !filtered.is_empty() => {
                    let index = filtered[self.cursor];
                    self.checked[index] = !self.checked[index];
                }
                Key::Enter => {
                    if self.checked.iter().any(|&value| value) {
                        break Ok(());
                    }
                    break Err(ui::Cancelled.into());
                }
                Key::Escape => break Err(ui::Cancelled.into()),
                Key::Backspace => {
                    self.query.pop();
                    self.cursor = 0;
                }
                Key::Char(character)
                    if !character.is_control() && character != ' ' && character != '\t' =>
                {
                    self.query.push(character);
                    self.cursor = 0;
                }
                _ => {}
            }
        };

        if drawn > 0 {
            term.clear_last_lines(drawn)?;
            term.flush()?;
        }

        result?;
        let selection = self
            .paths
            .iter()
            .enumerate()
            .filter(|(index, _)| self.checked[*index])
            .map(|(_, path)| path.clone())
            .collect();
        Ok(selection)
    }

    fn render(&self, filtered: &[usize], max_visible: usize, cols: usize) -> Vec<String> {
        let mut lines = Vec::new();
        let hint = if cols >= 96 {
            "(type to filter · ↑/↓ move · space toggle · Enter confirm · Esc cancel)"
        } else if cols >= 64 {
            "(type to filter · space toggle · Esc cancel)"
        } else {
            "(Esc cancels)"
        };
        lines.push(format!("{} {}", ui::accent("Select files"), ui::dim(hint)));

        let label_width = cols.saturating_sub(10).max(12);
        let start = filtered.len().saturating_sub(max_visible).min(self.cursor);
        let end = (start + max_visible).min(filtered.len());

        for (position, &index) in filtered[start..end].iter().enumerate() {
            let absolute = start + position;
            let checked = self.checked[index];
            let marker = if checked { "[x]" } else { "[ ]" };
            let label = truncate_chars(&self.labels[index], label_width);
            let text = format!("{marker} {label}");
            if absolute == self.cursor {
                lines.push(format!(
                    "❯ {}",
                    if checked {
                        ui::success(&text)
                    } else {
                        ui::accent(&text)
                    }
                ));
            } else if checked {
                lines.push(format!("  {}", ui::success(&text)));
            } else {
                lines.push(format!("  {text}"));
            }
        }

        if filtered.is_empty() {
            lines.push(ui::dim("  (no files match)"));
        }

        let selected = self.checked.iter().filter(|&&value| value).count();
        let summary = if selected > 0 {
            ui::success(format!("{selected} selected"))
        } else {
            ui::dim("nothing selected")
        };
        let query_width = cols.saturating_sub(32).max(8);
        lines.push(format!(
            "{} {}▌  {}",
            ui::dim("filter:"),
            ui::accent(truncate_chars(&self.query, query_width)),
            summary
        ));
        lines
    }
}

fn truncate_chars(value: &str, width: usize) -> String {
    if value.chars().count() <= width {
        return value.to_owned();
    }
    let mut truncated: String = value.chars().take(width.saturating_sub(1)).collect();
    truncated.push('…');
    truncated
}
