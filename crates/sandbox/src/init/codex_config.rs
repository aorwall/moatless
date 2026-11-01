//! Default `config.toml` for the Codex CLI.
//!
//! Disables the auto-injected `codex_apps` MCP server (OpenAI-hosted;
//! `github_*` tools 404 against our repos). The agent uses the already-
//! authenticated `gh` CLI instead — see `agent::options::GITHUB_CLI_HINT`.
//! `project_doc_max_bytes = 0` turns off Codex's `AGENTS.md` discovery, since
//! `agent::options::build_repo_instructions_section` already embeds each repo's
//! root instructions in the developer instructions.

use std::path::Path;

use tokio::fs;
use tracing::{info, warn};

// `project_doc_max_bytes` is a top-level key, so it must precede the `[features]`
// table header — a bare key written after the header would parse as
// `features.project_doc_max_bytes` instead.
const CONFIG_TOML: &str = "project_doc_max_bytes = 0\n\n[features]\napps = false\n";

/// The backend's model gateway, where Codex sends its model requests, keyed by
/// the task token in `token_file`.
///
/// Read through `auth.command` rather than `env_key`: Codex reruns the command
/// after a 401, so a token reminted while the app-server runs (a driver switch)
/// is picked up, where an env var keeps the refused one.
pub struct ModelRoute<'a> {
    pub base_url: &'a str,
    pub token_file: &'a Path,
}

/// Provider id for the model gateway.
///
/// Must not be a built-in id: Codex rejects reserved ids, and merges
/// `[model_providers.*]` with `or_insert`, so a built-in name would be silently
/// ignored rather than overridden.
const GATEWAY_PROVIDER_ID: &str = "moatless";

/// The model gateway's OpenAI root on the backend, which Codex appends
/// `/responses` to (`moatless_backend::model_gateway::openai`).
pub fn gateway_base_url(backend_api_url: &str) -> String {
    format!("{}/llm/openai/v1", backend_api_url.trim_end_matches('/'))
}

/// Render `config.toml`, routing the model provider when `route` is set.
///
/// Codex has no env-var equivalent of `ANTHROPIC_BASE_URL`; a custom provider
/// points it elsewhere, and `requires_openai_auth = false` keeps it off
/// `auth.json`. `pub` for `agent_transport_e2e.rs`.
pub fn render_config_toml(route: Option<&ModelRoute<'_>>) -> String {
    let Some(ModelRoute {
        base_url,
        token_file,
    }) = route
    else {
        return CONFIG_TOML.to_string();
    };
    // Codex refuses `auth` beside `env_key`.
    format!(
        "project_doc_max_bytes = 0\nmodel_provider = \"{GATEWAY_PROVIDER_ID}\"\n\n\
             [features]\napps = false\n\n\
             [model_providers.{GATEWAY_PROVIDER_ID}]\n\
             name = \"Moatless\"\n\
             base_url = \"{}\"\n\
             wire_api = \"responses\"\n\
             requires_openai_auth = false\n\n\
             [model_providers.{GATEWAY_PROVIDER_ID}.auth]\n\
             command = \"cat\"\n\
             args = [\"{}\"]\n\
             refresh_interval_ms = 0\n",
        toml_escape(base_url),
        toml_escape(&token_file.display().to_string())
    )
}

/// Basic strings are the only quoted form TOML accepts here, and both
/// characters escaped are legal in a URL or a path.
fn toml_escape(value: &str) -> String {
    value.replace('\\', r"\\").replace('"', "\\\"")
}

/// Write the Codex `config.toml` into `codex_home_dir`.
///
/// Failure is logged, not fatal: Codex still runs, just with the broken
/// `codex_apps` server re-enabled.
pub(super) async fn write_codex_config(codex_home_dir: &str, route: Option<&ModelRoute<'_>>) {
    if let Err(e) = fs::create_dir_all(codex_home_dir).await {
        warn!(error = %e, dir = %codex_home_dir, "Failed to create Codex home dir for config.toml");
        return;
    }

    let path = format!("{codex_home_dir}/config.toml");
    match fs::write(&path, render_config_toml(route)).await {
        Ok(()) => info!(path = %path, routed = route.is_some(), "Wrote Codex config.toml"),
        Err(e) => warn!(error = %e, path = %path, "Failed to write Codex config.toml"),
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use std::path::Path;

    use super::{
        CONFIG_TOML, GATEWAY_PROVIDER_ID, ModelRoute, render_config_toml, write_codex_config,
    };

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

    #[test]
    fn unrouted_config_is_unchanged() {
        assert_eq!(render_config_toml(None), CONFIG_TOML);
    }

    #[test]
    fn base_url_quotes_are_escaped() {
        let rendered = render_config_toml(Some(&ModelRoute {
            base_url: r#"http://h/"x"#,
            token_file: Path::new("/run/token"),
        }));
        assert!(rendered.contains(r#"base_url = "http://h/\"x""#));
    }

    #[tokio::test]
    async fn routes_to_the_gateway_with_the_task_token_file() {
        let tmp = TempDir::new().unwrap();
        let codex_home = tmp.path().join(".codex");

        write_codex_config(
            &codex_home.display().to_string(),
            Some(&ModelRoute {
                base_url: &super::gateway_base_url("http://backend:8080/"),
                token_file: Path::new("/run/moatless/.user_auth_token"),
            }),
        )
        .await;

        let contents = tokio::fs::read_to_string(codex_home.join("config.toml"))
            .await
            .unwrap();
        assert!(contents.contains(&format!("model_provider = \"{GATEWAY_PROVIDER_ID}\"")));
        assert!(contents.contains("base_url = \"http://backend:8080/llm/openai/v1\""));
        assert!(contents.contains("requires_openai_auth = false"));
        assert!(!contents.contains("env_key"));
        assert!(contents.contains(&format!(
            "[model_providers.{GATEWAY_PROVIDER_ID}.auth]\n\
             command = \"cat\"\n\
             args = [\"/run/moatless/.user_auth_token\"]\n\
             refresh_interval_ms = 0\n"
        )));
        let first_header = contents.find('[').unwrap();
        assert!(contents.find("model_provider").unwrap() < first_header);
    }
}
