//! Codex's `config.toml` for a test run against an OpenAI-compatible hub
//! keyed by `OPENAI_API_KEY`, where a pod would reach the backend's gateway.

/// `config.toml` for a hub at `base_url`, or the pod's unrouted default
/// without one, when Codex sends every turn to api.openai.com on `auth.json`.
pub fn config_toml(base_url: Option<&str>) -> String {
    let Some(base_url) = base_url else {
        return moatless_sandbox::init::codex_config::render_config_toml(None);
    };
    format!(
        "project_doc_max_bytes = 0\nmodel_provider = \"cliproxy\"\n\n\
         [features]\napps = false\n\n\
         [model_providers.cliproxy]\n\
         name = \"CLIProxyAPI\"\n\
         base_url = \"{base_url}\"\n\
         env_key = \"OPENAI_API_KEY\"\n\
         wire_api = \"responses\"\n\
         requires_openai_auth = false\n"
    )
}
