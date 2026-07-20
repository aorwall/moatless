//! Provider settings integration tests.

use integration_tests::mock_github::{MockGithub, MOCK_LOGIN};
use integration_tests::{base_url, query_psql, raw_client_as_user};
use moatless_api_schemas::provider_settings::{CodexConfigResponse, SaveCodexAuthRequest};
use pretty_assertions::assert_eq;

/// Fixed port the backend's `GITHUB_API_BASE_URL` points at in compose.
const DEFAULT_MOCK_PORT: u16 = 18092;

#[tokio::test]
async fn codex_provider_settings_crud() {
    let http = raw_client_as_user("codex-settings-user", "codex-user", "user");
    let base_url = base_url();

    let save_response = http
        .put(format!("{base_url}/api/v1/settings/codex/config"))
        .json(&SaveCodexAuthRequest {
            auth_json: r#"{"OPENAI_API_KEY":"sk-test","tokens":{"access_token":"session-token"}}"#
                .to_string(),
        })
        .send()
        .await
        .unwrap();
    assert_eq!(save_response.status().as_u16(), 200);
    let saved: CodexConfigResponse = save_response.json().await.unwrap();
    assert!(saved.connected);
    assert_eq!(saved.token_type.as_deref(), Some("auth_json"));

    let get_response = http
        .get(format!("{base_url}/api/v1/settings/codex/config"))
        .send()
        .await
        .unwrap();
    assert_eq!(get_response.status().as_u16(), 200);
    let fetched: CodexConfigResponse = get_response.json().await.unwrap();
    assert!(fetched.connected);
    assert_eq!(fetched.token_type.as_deref(), Some("auth_json"));

    let delete_response = http
        .delete(format!("{base_url}/api/v1/settings/codex/config"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete_response.status().as_u16(), 200);
    let deleted: CodexConfigResponse = delete_response.json().await.unwrap();
    assert!(!deleted.connected);
    assert_eq!(deleted.token_type, None);
}

// =============================================================================
// GitHub PAT — encrypted at rest
// =============================================================================

/// The point of the reference-row model: after saving a PAT, the plaintext must
/// exist nowhere in `provider_tokens`, and the config endpoint must still
/// report the connection without calling GitHub again.
#[tokio::test]
#[ignore] // Requires GITHUB_PROXY_E2E=1 + GITHUB_API_BASE_URL on the backend
async fn github_pat_is_stored_encrypted_and_never_in_provider_tokens() {
    if std::env::var("GITHUB_PROXY_E2E").is_err() {
        eprintln!("Skipping: GITHUB_PROXY_E2E not set");
        return;
    }

    let mock_port: u16 = std::env::var("GITHUB_MOCK_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_MOCK_PORT);
    let _mock = MockGithub::start(mock_port).await;

    let user_id = "github-pat-user";
    let http = raw_client_as_user(user_id, "github-pat", "user");
    let base_url = base_url();
    // Distinctive so a substring search over the row is meaningful.
    let pat = "ghp_integration_test_plaintext_marker";

    let save = http
        .put(format!("{base_url}/api/v1/settings/github/pat"))
        .json(&serde_json::json!({ "token": pat }))
        .send()
        .await
        .unwrap();
    assert_eq!(save.status().as_u16(), 200, "PAT save should succeed");
    let saved: serde_json::Value = save.json().await.unwrap();
    assert_eq!(saved["connected"], serde_json::json!(true));
    assert_eq!(saved["login"], serde_json::json!(MOCK_LOGIN));
    assert_eq!(saved["tokenType"], serde_json::json!("pat"));

    // ---- The row is a reference, not a credential ----
    let row = query_psql(&format!(
        "SELECT access_token || '|' || auth_method || '|' || COALESCE(token_type, '') \
         FROM provider_tokens WHERE user_id = '{user_id}' AND provider = 'github'"
    ));
    assert_eq!(row, "<secret-ref>|secret_ref|pat");

    // Belt and braces: the token must not appear anywhere on the row.
    let leaked = query_psql(&format!(
        "SELECT count(*) FROM provider_tokens \
         WHERE user_id = '{user_id}' AND to_jsonb(provider_tokens)::text LIKE '%{pat}%'"
    ));
    assert_eq!(leaked, "0", "PAT plaintext leaked into provider_tokens");

    // ---- The value lives encrypted in `secrets` ----
    let secret_row = query_psql(&format!(
        "SELECT kind || '|' || COALESCE(provider, '') || '|' || (value_ciphertext IS NOT NULL) \
         FROM secrets WHERE scope_type = 'user' AND scope_id = '{user_id}' AND key = 'GITHUB_TOKEN'"
    ));
    assert_eq!(secret_row, "provider_token|github|true");

    let cipher_leak = query_psql(&format!(
        "SELECT count(*) FROM secrets \
         WHERE scope_id = '{user_id}' AND key = 'GITHUB_TOKEN' \
           AND encode(value_ciphertext, 'escape') LIKE '%{pat}%'"
    ));
    assert_eq!(cipher_leak, "0", "ciphertext contains the plaintext PAT");

    // ---- The config endpoint still reports the connection ----
    let config = http
        .get(format!("{base_url}/api/v1/settings/github/config"))
        .send()
        .await
        .unwrap();
    assert_eq!(config.status().as_u16(), 200);
    let config: serde_json::Value = config.json().await.unwrap();
    assert_eq!(config["connected"], serde_json::json!(true));
    assert_eq!(config["login"], serde_json::json!(MOCK_LOGIN));
    assert_eq!(config["tokenType"], serde_json::json!("pat"));

    // ---- The PAT is hidden from the generic secrets surface ----
    let secrets = http
        .get(format!("{base_url}/api/v1/secrets?scope=user"))
        .send()
        .await
        .unwrap();
    assert_eq!(secrets.status().as_u16(), 200);
    let body = secrets.text().await.unwrap();
    assert!(
        !body.contains("GITHUB_TOKEN"),
        "provider_token secrets must not be listed in the secrets UI: {body}"
    );

    // ---- Disconnecting removes the secret too ----
    let disconnect = http
        .delete(format!("{base_url}/api/v1/settings/github/config"))
        .send()
        .await
        .unwrap();
    assert_eq!(disconnect.status().as_u16(), 200);
    let remaining = query_psql(&format!(
        "SELECT count(*) FROM secrets WHERE scope_id = '{user_id}' AND key = 'GITHUB_TOKEN'"
    ));
    assert_eq!(remaining, "0", "secret outlived the GitHub disconnect");
}
