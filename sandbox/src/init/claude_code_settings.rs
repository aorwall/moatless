//! Default Claude Code `settings.json` for the sandbox.
//!
//! Writes a known-good settings file at `$CLAUDE_CONFIG_DIR/settings.json`
//! so every sandbox boots with the same Claude CLI configuration.

use tokio::fs;
use tracing::{info, warn};

/// Absolute `CLAUDE.md` paths to exclude from Claude's memory loading — the root
/// instruction file of every checked-out repo. Those instructions are instead
/// embedded in the agent's system prompt (see [`crate::agent::options`]), so
/// listing them here stops Claude pulling the same file in a second time when the
/// agent opens a file inside that directory. Claude's `claudeMdExcludes` matches
/// absolute file paths.
pub(super) fn claude_md_excludes(repo_dirs: &[String]) -> Vec<String> {
    repo_dirs
        .iter()
        .map(|dir| format!("{}/CLAUDE.md", dir.trim_end_matches('/')))
        .collect()
}

/// Write the default Claude Code settings file into `claude_config_dir`.
///
/// `claude_md_excludes` is the absolute `CLAUDE.md` paths to keep out of memory
/// loading (see [`claude_md_excludes`]). Always overwrites — the values are
/// static defaults we own. Failure is logged but never fatal: the Claude CLI
/// still works without this file, it just won't pick up our preferred defaults.
pub(super) async fn write_claude_code_settings(
    claude_config_dir: &str,
    claude_md_excludes: &[String],
) {
    let path = format!("{claude_config_dir}/settings.json");
    let payload = serde_json::json!({
        "env": {
            "CLAUDE_CODE_AUTO_COMPACT_WINDOW": "300000",
            "CLAUDE_CODE_DISABLE_ADAPTIVE_THINKING": "1",
            "CLAUDE_CODE_DISABLE_AUTO_MEMORY": "1",
            "CLAUDE_CODE_SUBAGENT_MODEL": "sonnet",
        },
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

    #[tokio::test]
    async fn writes_expected_env_settings() {
        let tmp = TempDir::new().unwrap();
        let claude_config_dir = tmp.path().join(".claude");
        tokio::fs::create_dir_all(&claude_config_dir).await.unwrap();

        write_claude_code_settings(&claude_config_dir.display().to_string(), &[]).await;

        let contents = tokio::fs::read_to_string(claude_config_dir.join("settings.json"))
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&contents).unwrap();

        let env = parsed.get("env").and_then(|v| v.as_object()).unwrap();
        assert_eq!(
            env.get("CLAUDE_CODE_AUTO_COMPACT_WINDOW").unwrap(),
            "300000"
        );
        assert_eq!(
            env.get("CLAUDE_CODE_DISABLE_ADAPTIVE_THINKING").unwrap(),
            "1"
        );
        assert_eq!(env.get("CLAUDE_CODE_DISABLE_AUTO_MEMORY").unwrap(), "1");
        assert_eq!(env.get("CLAUDE_CODE_SUBAGENT_MODEL").unwrap(), "sonnet");
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
        assert!(parsed.get("env").is_some());
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
        let dirs = vec![
            "/workspace/api".to_string(),
            "/workspace/web/".to_string(),
        ];
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
