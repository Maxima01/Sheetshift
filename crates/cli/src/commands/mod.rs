//! Subcommand implementations. Clap definitions live beside each command.

pub mod convert;
pub mod history;
pub mod info;
pub mod settings;

use anyhow::{bail, Context, Result};
use std::path::PathBuf;

pub use convert::ConvertArgs;
pub use history::HistoryArgs;
pub use info::{FormatsArgs, TermsArgs};
pub use settings::SettingsCommand;

/// Resolve a possibly relative path to an absolute Unicode string.
pub fn absolute_path_string(path: PathBuf) -> Result<String> {
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
