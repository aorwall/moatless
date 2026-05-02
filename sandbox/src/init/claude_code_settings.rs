//! Default Claude Code `settings.json` for the sandbox.
//!
//! Writes a known-good settings file at `$CLAUDE_CONFIG_DIR/settings.json`
//! so every sandbox boots with the same Claude CLI configuration.

use tokio::fs;
use tracing::{info, warn};

/// Write the default Claude Code settings file into `claude_config_dir`.
///
/// Always overwrites — the values are static defaults we own. Failure is
/// logged but never fatal: the Claude CLI still works without this file,
/// it just won't pick up our preferred env defaults.
pub(super) async fn write_claude_code_settings(claude_config_dir: &str) {
    let path = format!("{claude_config_dir}/settings.json");
    let payload = serde_json::json!({
        "env": {
            "CLAUDE_CODE_AUTO_COMPACT_WINDOW": "300000",
            "CLAUDE_CODE_DISABLE_ADAPTIVE_THINKING": "1",
            "CLAUDE_CODE_DISABLE_AUTO_MEMORY": "1",
            "CLAUDE_CODE_SUBAGENT_MODEL": "sonnet",
        }
    });

    match fs::write(&path, payload.to_string()).await {
        Ok(()) => info!(path = %path, "Wrote Claude Code settings file"),
        Err(e) => warn!(error = %e, path = %path, "Failed to write Claude Code settings file"),
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::write_claude_code_settings;

    #[tokio::test]
    async fn writes_expected_env_settings() {
        let tmp = TempDir::new().unwrap();
        let claude_config_dir = tmp.path().join(".claude");
        tokio::fs::create_dir_all(&claude_config_dir).await.unwrap();

        write_claude_code_settings(&claude_config_dir.display().to_string()).await;

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

        write_claude_code_settings(&claude_config_dir.display().to_string()).await;

        let contents = tokio::fs::read_to_string(claude_config_dir.join("settings.json"))
            .await
            .unwrap();
        assert_ne!(contents, "stale");
        let parsed: serde_json::Value = serde_json::from_str(&contents).unwrap();
        assert!(parsed.get("env").is_some());
    }
}
