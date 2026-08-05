//! Default `config.toml` for the Codex CLI.
//!
//! Disables the auto-injected `codex_apps` MCP server (an OpenAI-hosted
//! backend whose `github_*` tools authenticate via ChatGPT OAuth and return
//! 404 against our repos). The agent uses the already-authenticated `gh`
//! CLI for GitHub operations instead — see `agent::options::GITHUB_CLI_HINT`.
//!
//! `project_doc_max_bytes = 0` turns off Codex's automatic `AGENTS.md`
//! discovery: each repo's root instructions are embedded in the developer
//! instructions instead (see `agent::options::build_repo_instructions_section`),
//! so letting Codex load them again from the working directory would duplicate
//! them.

use tokio::fs;
use tracing::{info, warn};

// `project_doc_max_bytes` is a top-level key, so it must precede the `[features]`
// table header — a bare key written after the header would parse as
// `features.project_doc_max_bytes` instead.
const CONFIG_TOML: &str = "project_doc_max_bytes = 0\n\n[features]\napps = false\n";

/// Write the default Codex `config.toml` into `codex_home_dir`.
///
/// Always overwrites — the values are static defaults we own. Failure is
/// logged but never fatal: Codex still runs without this file, it just
/// re-enables the broken `codex_apps` server.
pub(super) async fn write_codex_config(codex_home_dir: &str) {
    if let Err(e) = fs::create_dir_all(codex_home_dir).await {
        warn!(error = %e, dir = %codex_home_dir, "Failed to create Codex home dir for config.toml");
        return;
    }

    let path = format!("{codex_home_dir}/config.toml");
    match fs::write(&path, CONFIG_TOML).await {
        Ok(()) => info!(path = %path, "Wrote Codex config.toml"),
        Err(e) => warn!(error = %e, path = %path, "Failed to write Codex config.toml"),
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::{write_codex_config, CONFIG_TOML};

    #[tokio::test]
    async fn writes_expected_config() {
        let tmp = TempDir::new().unwrap();
        let codex_home = tmp.path().join(".codex");

        write_codex_config(&codex_home.display().to_string()).await;

        let contents = tokio::fs::read_to_string(codex_home.join("config.toml"))
            .await
            .unwrap();
        assert_eq!(contents, CONFIG_TOML);
        assert!(contents.contains("apps = false"));
        assert!(contents.contains("project_doc_max_bytes = 0"));
        // `project_doc_max_bytes` is a top-level key, so it must appear before the
        // `[features]` header — otherwise TOML nests it as `features.*`.
        let doc_idx = contents.find("project_doc_max_bytes").unwrap();
        let features_idx = contents.find("[features]").unwrap();
        assert!(doc_idx < features_idx);
    }

    #[tokio::test]
    async fn overwrites_existing_config() {
        let tmp = TempDir::new().unwrap();
        let codex_home = tmp.path().join(".codex");
        tokio::fs::create_dir_all(&codex_home).await.unwrap();
        tokio::fs::write(codex_home.join("config.toml"), "stale = true\n")
            .await
            .unwrap();

        write_codex_config(&codex_home.display().to_string()).await;

        let contents = tokio::fs::read_to_string(codex_home.join("config.toml"))
            .await
            .unwrap();
        assert_eq!(contents, CONFIG_TOML);
    }

    #[tokio::test]
    async fn non_fatal_when_path_unwritable() {
        // Point at a path where a regular file blocks directory creation.
        let tmp = TempDir::new().unwrap();
        let blocker = tmp.path().join("not-a-dir");
        tokio::fs::write(&blocker, "").await.unwrap();
        let nested = blocker.join("nested");

        // Must not panic; the warn! branch handles the error.
        write_codex_config(&nested.display().to_string()).await;
    }
}
