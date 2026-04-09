//! Provider settings integration tests.

use integration_tests::{base_url, raw_client_as_user};
use moatless_api_schemas::provider_settings::{CodexConfigResponse, SaveCodexAuthRequest};
use pretty_assertions::assert_eq;

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
