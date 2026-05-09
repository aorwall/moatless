//! Raw HTTP helpers for endpoints not covered by the generated OpenAPI client.
//!
//! The generated `moatless_api_client::Client` covers most endpoints. This module
//! provides helpers for operations that need raw HTTP access (e.g., webhook inbound
//! with custom headers, repository creation with scope fields).

use crate::base_url;
use serde_json::Value;

/// Send a webhook inbound message. Returns the raw response.
///
/// POST /api/v1/integrations/webhook/{trigger_id}
pub async fn send_webhook_message(
    http: &reqwest::Client,
    trigger_id: &str,
    payload: &Value,
) -> reqwest::Response {
    http.post(format!(
        "{}/api/v1/integrations/webhook/{}",
        base_url(),
        trigger_id
    ))
    .json(payload)
    .send()
    .await
    .unwrap()
}

/// Send a webhook inbound message with extra headers (e.g., X-Webhook-Signature).
pub async fn send_webhook_message_with_headers(
    http: &reqwest::Client,
    trigger_id: &str,
    payload: &Value,
    extra_headers: &[(String, String)],
) -> reqwest::Response {
    let mut req = http
        .post(format!(
            "{}/api/v1/integrations/webhook/{}",
            base_url(),
            trigger_id
        ))
        .json(payload);

    for (key, value) in extra_headers {
        req = req.header(key.as_str(), value.as_str());
    }

    req.send().await.unwrap()
}

/// Create a repository with scope fields (not in the standard OpenAPI schema).
///
/// POST /api/v1/repositories with extra fields: scope, team_id
pub async fn create_repository_with_scope(
    http: &reqwest::Client,
    payload: &Value,
) -> reqwest::Response {
    http.post(format!("{}/api/v1/repositories", base_url()))
        .json(payload)
        .send()
        .await
        .unwrap()
}

/// Verify auth endpoint. Returns raw response so caller can check status/headers.
///
/// GET /api/v1/auth/verify
pub async fn verify_auth(http: &reqwest::Client) -> reqwest::Response {
    http.get(format!("{}/api/v1/auth/verify", base_url()))
        .send()
        .await
        .unwrap()
}

/// Verify auth with extra headers (e.g., X-Forwarded-Host).
pub async fn verify_auth_with_headers(
    http: &reqwest::Client,
    extra_headers: &[(String, String)],
) -> reqwest::Response {
    let mut req = http.get(format!("{}/api/v1/auth/verify", base_url()));

    for (key, value) in extra_headers {
        req = req.header(key.as_str(), value.as_str());
    }

    req.send().await.unwrap()
}

/// Update a task trigger (PUT).
///
/// PUT /api/v1/triggers/{id}
pub async fn update_task_trigger(
    http: &reqwest::Client,
    trigger_id: &str,
    payload: &Value,
) -> reqwest::Response {
    http.put(format!("{}/api/v1/triggers/{}", base_url(), trigger_id))
        .json(payload)
        .send()
        .await
        .unwrap()
}

/// Create a task route and return raw response.
///
/// POST /api/v1/task-routes
pub async fn create_task_route_raw(http: &reqwest::Client, payload: &Value) -> reqwest::Response {
    http.post(format!("{}/api/v1/task-routes", base_url()))
        .json(payload)
        .send()
        .await
        .unwrap()
}

/// Send a task reply and return raw response.
///
/// POST /api/v1/tasks/{task_id}/reply
pub async fn send_task_reply_raw(
    http: &reqwest::Client,
    task_id: &str,
    payload: &Value,
) -> reqwest::Response {
    http.post(format!("{}/api/v1/tasks/{}/reply", base_url(), task_id))
        .json(payload)
        .send()
        .await
        .unwrap()
}

/// Send a task reply to a specific binding and return raw response.
pub async fn send_task_reply_to_binding_raw(
    http: &reqwest::Client,
    task_id: &str,
    binding_id: &str,
    text: &str,
) -> reqwest::Response {
    http.post(format!("{}/api/v1/tasks/{}/reply", base_url(), task_id))
        .json(&serde_json::json!({
            "text": text,
            "bindingId": binding_id,
            "fileIds": []
        }))
        .send()
        .await
        .unwrap()
}

/// Create a task and return raw response (for testing error cases).
///
/// POST /api/v1/tasks
pub async fn create_task_raw(http: &reqwest::Client, payload: &Value) -> reqwest::Response {
    http.post(format!("{}/api/v1/tasks", base_url()))
        .json(payload)
        .send()
        .await
        .unwrap()
}

/// Send a message to a task and return raw response.
///
/// POST /api/v1/tasks/{task_id}/messages
pub async fn send_message_raw(
    http: &reqwest::Client,
    task_id: &str,
    payload: &Value,
) -> reqwest::Response {
    http.post(format!("{}/api/v1/tasks/{}/messages", base_url(), task_id))
        .json(payload)
        .send()
        .await
        .unwrap()
}

/// Delete a task and return raw response.
///
/// DELETE /api/v1/tasks/{task_id}
pub async fn delete_task_raw(http: &reqwest::Client, task_id: &str) -> reqwest::Response {
    http.delete(format!("{}/api/v1/tasks/{}", base_url(), task_id))
        .send()
        .await
        .unwrap()
}

/// Create team credential (upsert). Returns raw response for error testing.
///
/// PUT /api/v1/teams/{team_id}/credentials/{provider}
pub async fn upsert_team_credential(
    http: &reqwest::Client,
    team_id: &str,
    provider: &str,
    payload: &Value,
) -> reqwest::Response {
    http.put(format!(
        "{}/api/v1/teams/{}/credentials/{}",
        base_url(),
        team_id,
        provider
    ))
    .json(payload)
    .send()
    .await
    .unwrap()
}

/// Create a team and return raw response (for testing error cases like duplicate slug).
///
/// POST /api/v1/teams
pub async fn create_team_raw(http: &reqwest::Client, payload: &Value) -> reqwest::Response {
    http.post(format!("{}/api/v1/teams", base_url()))
        .json(payload)
        .send()
        .await
        .unwrap()
}
