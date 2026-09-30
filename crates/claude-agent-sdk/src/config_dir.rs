//! Where Claude's config directory and the user's home directory are.

use crate::api::options::ClaudeOptions;
use std::path::PathBuf;

/// The user's home directory, empty when the platform names none.
pub(crate) fn home_dir() -> PathBuf {
    std::env::home_dir().unwrap_or_default()
}

/// Claude's config directory: `options.config_dir`, else `<home>/.claude`.
pub(crate) fn claude_config_dir(options: &ClaudeOptions) -> PathBuf {
    options
        .config_dir
        .as_ref()
        .map_or_else(|| home_dir().join(".claude"), PathBuf::from)
}
