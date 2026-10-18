//! Animated spinners and multi-segment progress bars for conversion batches.
//!
//! Everything renders on stderr through indicatif so `--json` stdout stays
//! pure. In quiet mode (JSON output or non-TTY sessions) no bars are built.

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::time::Duration;

const SPINNER_TICKS: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

const BAR_TEMPLATE: &str =
    "{prefix} {bar:32.cyan/blue} {pos}/{len} {percent:>3}% {elapsed_precise} {msg}";
const SPINNER_TEMPLATE: &str = "{spinner:.green} {wide_msg}";

/// Two coordinated bars: an animated per-file spinner over an overall
/// batch progress bar with elapsed time, percentage, and status text.
pub struct BatchProgress {
    bars: Option<Bars>,
    total: usize,
    completed: usize,
    failures: usize,
}

struct Bars {
    spinner: ProgressBar,
    overall: ProgressBar,
}

impl BatchProgress {
    /// `active` is false for `--json` runs and non-interactive sessions.
    pub fn new(active: bool, total: usize) -> Self {
        let bars = active.then(|| {
            let multi = MultiProgress::new();
            let overall = ProgressBar::new(total as u64)
                .with_style(
                    ProgressStyle::with_template(BAR_TEMPLATE)
                        .unwrap_or_else(|_| ProgressStyle::default_bar()),
                )
                .with_prefix("convert")
                .with_message("queued");
            let spinner = ProgressBar::new_spinner()
                .with_style(
                    ProgressStyle::with_template(SPINNER_TEMPLATE)
                        .unwrap_or_else(|_| ProgressStyle::default_spinner())
                        .tick_strings(SPINNER_TICKS),
                )
                .with_message("starting");
            let spinner = multi.add(spinner);
            spinner.enable_steady_tick(Duration::from_millis(80));
            let overall = multi.add(overall);
            Bars { spinner, overall }
        });
        Self {
            bars,
            total,
            completed: 0,
            failures: 0,
        }
    }

    /// Show the file that is about to be converted.
    pub fn start_file(&self, index: usize, name: &str) {
        if let Some(bars) = &self.bars {
            bars.spinner
                .set_message(format!("converting {name} ({}/{})", index + 1, self.total));
            bars.overall.set_message(format!("working on {name}"));
        }
    }

    /// Record a finished file and advance the batch bar.
    pub fn finish_file(&mut self, name: &str, success: bool) {
        self.completed += 1;
        if !success {
            self.failures += 1;
        }
        if let Some(bars) = &self.bars {
            bars.overall.set_position(self.completed as u64);
            if success {
                bars.overall.set_message(format!("converted {name}"));
            } else {
                bars.overall
                    .set_message(format!("failed {name} ({} failed)", self.failures));
            }
        }
    }

    /// Clear the spinner and leave the completed batch bar in a final state.
    pub fn finish(self) {
        if let Some(bars) = self.bars {
            bars.spinner.finish_and_clear();
            let status = if self.failures == 0 {
                format!("{} converted", self.completed)
            } else {
                format!("{} converted, {} failed", self.completed, self.failures)
            };
            bars.overall.finish_with_message(status);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bar_templates_parse() {
        assert!(ProgressStyle::with_template(BAR_TEMPLATE).is_ok());
        assert!(ProgressStyle::with_template(SPINNER_TEMPLATE).is_ok());
    }

    #[test]
    fn quiet_progress_is_a_no_op() {
        let progress = BatchProgress::new(false, 3);
        progress.start_file(0, "a.txt");
        let mut progress = progress;
        progress.finish_file("a.txt", true);
        progress.finish();
    }
}
