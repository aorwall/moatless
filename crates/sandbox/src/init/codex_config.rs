//! Default `config.toml` for the Codex CLI.
//!
//! Disables the auto-injected `codex_apps` MCP server (OpenAI-hosted;
//! `github_*` tools 404 against our repos). The agent uses the already-
//! authenticated `gh` CLI instead — see `agent::options::GITHUB_CLI_HINT`.
//! `project_doc_max_bytes = 0` turns off Codex's `AGENTS.md` discovery, since
//! `agent::options::build_repo_instructions_section` already embeds each repo's
//! root instructions in the developer instructions.

use tokio::fs;
use tracing::{info, warn};

// `project_doc_max_bytes` is a top-level key, so it must precede the `[features]`
// table header — a bare key written after the header would parse as
// `features.project_doc_max_bytes` instead.
const CONFIG_TOML: &str = "project_doc_max_bytes = 0\n\n[features]\napps = false\n";

/// Provider id for the proxy hub.
///
/// Must not be a built-in id: Codex rejects reserved ids, and merges
/// `[model_providers.*]` with `or_insert`, so a built-in name would be silently
/// ignored rather than overridden.
const HUB_PROVIDER_ID: &str = "cliproxy";

/// Render `config.toml`, routing the model provider at `base_url` when set.
///
/// Codex has no env-var equivalent of `ANTHROPIC_BASE_URL`; a custom provider
/// points it at a proxy, and `requires_openai_auth = false` keeps the hub key
/// out of `auth.json` (see [`super::codex_watcher`]). `pub` for
/// `agent_transport_e2e.rs`.
pub fn render_config_toml(base_url: Option<&str>) -> String {
    let Some(base_url) = base_url else {
        return CONFIG_TOML.to_string();
    };

    // Basic strings are the only quoted form TOML accepts here, and both of
    // these are legal in a URL.
    let escaped = base_url.replace('\\', r"\\").replace('"', "\\\"");

    format!(
        "project_doc_max_bytes = 0\nmodel_provider = \"{HUB_PROVIDER_ID}\"\n\n\
         [features]\napps = false\n\n\
         [model_providers.{HUB_PROVIDER_ID}]\n\
         name = \"CLIProxyAPI\"\n\
         base_url = \"{escaped}\"\n\
         env_key = \"OPENAI_API_KEY\"\n\
         wire_api = \"responses\"\n\
         requires_openai_auth = false\n"
    )
}

/// Write the Codex `config.toml` into `codex_home_dir`.
///
/// Always overwrites the values we own. Failure is logged but non-fatal: Codex
/// still runs without this file, just with the broken `codex_apps` server
/// re-enabled.
pub(super) async fn write_codex_config(codex_home_dir: &str, base_url: Option<&str>) {
    if let Err(e) = fs::create_dir_all(codex_home_dir).await {
        warn!(error = %e, dir = %codex_home_dir, "Failed to create Codex home dir for config.toml");
        return;
    }

    let path = format!("{codex_home_dir}/config.toml");
    match fs::write(&path, render_config_toml(base_url)).await {
        Ok(()) => info!(path = %path, routed = base_url.is_some(), "Wrote Codex config.toml"),
        Err(e) => warn!(error = %e, path = %path, "Failed to write Codex config.toml"),
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::{CONFIG_TOML, HUB_PROVIDER_ID, render_config_toml, write_codex_config};

    #[tokio::test]
    async fn writes_expected_config() {
        let tmp = TempDir::new().unwrap();
        let codex_home = tmp.path().join(".codex");

        write_codex_config(&codex_home.display().to_string(), None).await;

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

        write_codex_config(&codex_home.display().to_string(), None).await;

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
        write_codex_config(&nested.display().to_string(), None).await;
    }

    #[tokio::test]
    async fn routes_to_the_hub_when_a_base_url_is_set() {
        let tmp = TempDir::new().unwrap();
        let codex_home = tmp.path().join(".codex");

        write_codex_config(
            &codex_home.display().to_string(),
            Some("http://hub.example:8317/v1"),
        )
        .await;

        let contents = tokio::fs::read_to_string(codex_home.join("config.toml"))
            .await
            .unwrap();
        assert!(contents.contains(&format!("model_provider = \"{HUB_PROVIDER_ID}\"")));
        assert!(contents.contains(&format!("[model_providers.{HUB_PROVIDER_ID}]")));
        assert!(contents.contains("base_url = \"http://hub.example:8317/v1\""));
        // The credential is named, never inlined.
        assert!(contents.contains("env_key = \"OPENAI_API_KEY\""));
        // auth.json is bypassed, so the watcher never sees the hub key.
        assert!(contents.contains("requires_openai_auth = false"));
        let first_header = contents.find('[').unwrap();
        assert!(contents.find("project_doc_max_bytes").unwrap() < first_header);
        assert!(contents.find("model_provider").unwrap() < first_header);
    }

    #[test]
    fn unrouted_config_is_unchanged() {
        assert_eq!(render_config_toml(None), CONFIG_TOML);
    }

    #[test]
    fn base_url_quotes_are_escaped() {
        let rendered = render_config_toml(Some(r#"http://h/"x"#));
        assert!(rendered.contains(r#"base_url = "http://h/\"x""#));
    }
}
