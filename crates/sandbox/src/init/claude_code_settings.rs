//! Default Claude Code `settings.json` for the sandbox.
//!
//! Writes a known-good settings file at `$CLAUDE_CONFIG_DIR/settings.json` so
//! every sandbox boots with the same Claude CLI configuration.
//!
//! Env-var defaults are not written here — see
//! [`crate::agent::options::CLAUDE_ENV_DEFAULTS`] for why.

use tokio::fs;
use tracing::{info, warn};

/// Absolute `CLAUDE.md` paths to exclude from Claude's memory loading.
///
/// Those instructions are already embedded in the system prompt (see
/// [`crate::agent::options`]), so this stops Claude loading the same file a
/// second time.
pub(super) fn claude_md_excludes(repo_dirs: &[String]) -> Vec<String> {
    repo_dirs
        .iter()
        .map(|dir| format!("{}/CLAUDE.md", dir.trim_end_matches('/')))
        .collect()
}

/// Write the default Claude Code settings file into `claude_config_dir`.
///
/// Always overwrites the static defaults we own. Failure is logged but
/// non-fatal: Claude CLI still runs without this file, just without our
/// preferred defaults.
pub(super) async fn write_claude_code_settings(
    claude_config_dir: &str,
    claude_md_excludes: &[String],
) {
    let path = format!("{claude_config_dir}/settings.json");
    let payload = serde_json::json!({
        "claudeMdExcludes": claude_md_excludes,
    });

    match fs::write(&path, payload.to_string()).await {
        Ok(()) => info!(path = %path, "Wrote Claude Code settings file"),
        Err(e) => warn!(error = %e, path = %path, "Failed to write Claude Code settings file"),
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::{claude_md_excludes, write_claude_code_settings};

    /// Env defaults belong to `agent::options::CLAUDE_ENV_DEFAULTS`, where a
    /// scoped value can outrank them. A settings `env` block would win over
    /// that channel, so this file must not carry one.
    #[tokio::test]
    async fn writes_no_env_block() {
        let tmp = TempDir::new().unwrap();
        let claude_config_dir = tmp.path().join(".claude");
        tokio::fs::create_dir_all(&claude_config_dir).await.unwrap();

        write_claude_code_settings(&claude_config_dir.display().to_string(), &[]).await;

        let contents = tokio::fs::read_to_string(claude_config_dir.join("settings.json"))
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&contents).unwrap();

        assert!(parsed.get("env").is_none());
    }

    #[tokio::test]
    async fn overwrites_existing_settings() {
        let tmp = TempDir::new().unwrap();
        let claude_config_dir = tmp.path().join(".claude");
        tokio::fs::create_dir_all(&claude_config_dir).await.unwrap();
        tokio::fs::write(claude_config_dir.join("settings.json"), "stale")
            .await
            .unwrap();

        write_claude_code_settings(&claude_config_dir.display().to_string(), &[]).await;

        let contents = tokio::fs::read_to_string(claude_config_dir.join("settings.json"))
            .await
            .unwrap();
        assert_ne!(contents, "stale");
        let parsed: serde_json::Value = serde_json::from_str(&contents).unwrap();
        assert!(parsed.get("claudeMdExcludes").is_some());
    }

    #[tokio::test]
    async fn writes_claude_md_excludes_into_settings() {
        let tmp = TempDir::new().unwrap();
        let claude_config_dir = tmp.path().join(".claude");
        tokio::fs::create_dir_all(&claude_config_dir).await.unwrap();
        let excludes = vec![
            "/workspace/api/CLAUDE.md".to_string(),
            "/workspace/web/CLAUDE.md".to_string(),
        ];

        write_claude_code_settings(&claude_config_dir.display().to_string(), &excludes).await;

        let contents = tokio::fs::read_to_string(claude_config_dir.join("settings.json"))
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&contents).unwrap();
        let listed: Vec<String> = parsed
            .get("claudeMdExcludes")
            .and_then(|v| v.as_array())
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        assert_eq!(listed, excludes);
    }

    #[test]
    fn claude_md_excludes_maps_repo_dirs_to_root_claude_md() {
        let dirs = vec!["/workspace/api".to_string(), "/workspace/web/".to_string()];
        assert_eq!(
            claude_md_excludes(&dirs),
            vec![
                "/workspace/api/CLAUDE.md".to_string(),
                "/workspace/web/CLAUDE.md".to_string(),
            ]
        );
    }

    #[test]
    fn claude_md_excludes_empty_when_no_repos() {
        assert!(claude_md_excludes(&[]).is_empty());
    }
}
