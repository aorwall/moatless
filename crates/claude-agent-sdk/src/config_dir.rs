//! Shared resolution of Claude's config directory, mirroring the Claude CLI.

use std::path::PathBuf;

/// Resolve Claude's config dir, mirroring the Claude CLI's precedence:
/// `CLAUDE_CONFIG_DIR` > `CLAUDE_HOME` (documented alias) > `<home_dir>/.claude`.
/// (docs: code.claude.com/docs/en/env-vars). `home_dir` is the caller's resolved
/// HOME so each caller keeps its own HOME-unset fallback.
pub fn resolve_claude_config_dir(home_dir: &str) -> PathBuf {
    std::env::var("CLAUDE_CONFIG_DIR")
        .or_else(|_| std::env::var("CLAUDE_HOME"))
        .map_or_else(|_| PathBuf::from(home_dir).join(".claude"), PathBuf::from)
}

/// Serializes tests across the crate that mutate the process-global
/// `CLAUDE_CONFIG_DIR` / `CLAUDE_HOME` env vars (here and in `tmux::session`),
/// since cargo runs them concurrently in one binary.
#[cfg(test)]
pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_claude_config_dir_follows_cli_precedence() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        std::env::remove_var("CLAUDE_CONFIG_DIR");
        std::env::remove_var("CLAUDE_HOME");

        // 1. Neither set → `<home>/.claude`.
        assert_eq!(
            resolve_claude_config_dir("/home/moatless"),
            PathBuf::from("/home/moatless/.claude")
        );

        // 2. CLAUDE_HOME used as an alias when CLAUDE_CONFIG_DIR is unset.
        std::env::set_var("CLAUDE_HOME", "/opt/claude-home");
        assert_eq!(
            resolve_claude_config_dir("/home/moatless"),
            PathBuf::from("/opt/claude-home")
        );

        // 3. CLAUDE_CONFIG_DIR (what the Claude CLI uses) wins over CLAUDE_HOME.
        std::env::set_var("CLAUDE_CONFIG_DIR", "/opt/moatless/.claude");
        assert_eq!(
            resolve_claude_config_dir("/home/moatless"),
            PathBuf::from("/opt/moatless/.claude")
        );

        std::env::remove_var("CLAUDE_CONFIG_DIR");
        std::env::remove_var("CLAUDE_HOME");
    }
}
