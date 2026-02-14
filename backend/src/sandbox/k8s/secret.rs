//! Kubernetes Secret lifecycle operations for user credentials.

use k8s_openapi::api::core::v1::Secret;
use kube::api::{Api, PostParams};
use std::collections::BTreeMap;
use tracing::{debug, info, warn};

use super::client::K8sClient;
use super::naming::{generate_secret_name, generate_user_secret_name};
use crate::sandbox::models::UserCredentials;

// =============================================================================
// Legacy task secret operations (cleanup only)
// =============================================================================

/// Delete a legacy task secret during workspace cleanup.
///
/// Kept only for migration — new provisions use per-user secrets.
pub async fn delete_task_secret(k8s: &K8sClient, task_id: &str) -> Result<(), kube::Error> {
    let secrets: Api<Secret> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let secret_name = generate_secret_name(task_id);

    match secrets.delete(&secret_name, &Default::default()).await {
        Ok(_) => {
            info!(task_id, secret_name = %secret_name, "Legacy task secret deleted");
            Ok(())
        }
        Err(kube::Error::Api(ref e)) if e.code == 404 => {
            debug!(task_id, "Legacy task secret not found, already deleted");
            Ok(())
        }
        Err(e) => {
            warn!(task_id, error = %e, "Failed to delete legacy task secret");
            Err(e)
        }
    }
}

// =============================================================================
// Per-user secret operations
// =============================================================================

/// Find existing secret for a user
pub async fn find_user_secret(k8s: &K8sClient, user_id: &str) -> Option<String> {
    let secrets: Api<Secret> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let secret_name = generate_user_secret_name(user_id);

    match secrets.get(&secret_name).await {
        Ok(_) => {
            debug!(user_id, secret_name = %secret_name, "Found existing user secret");
            Some(secret_name)
        }
        Err(_) => None,
    }
}

/// Create a Kubernetes Secret for user credentials.
///
/// Handles 409 Conflict (race condition) by falling back to update.
pub async fn create_user_secret(
    k8s: &K8sClient,
    user_id: &str,
    credentials: &UserCredentials,
) -> Result<String, kube::Error> {
    let secrets: Api<Secret> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let secret_name = generate_user_secret_name(user_id);

    let string_data = build_user_string_data(credentials);

    let mut labels = BTreeMap::new();
    labels.insert(
        "app.kubernetes.io/name".to_string(),
        "moatless-user-credentials".to_string(),
    );
    labels.insert(
        "app.kubernetes.io/managed-by".to_string(),
        "moatless".to_string(),
    );
    labels.insert("moatless.io/user-id".to_string(), user_id.to_string());

    let secret: Secret = serde_json::from_value(serde_json::json!({
        "apiVersion": "v1",
        "kind": "Secret",
        "metadata": {
            "name": secret_name,
            "namespace": k8s.namespace(),
            "labels": labels,
        },
        "type": "Opaque",
        "stringData": string_data,
    }))
    .map_err(kube::Error::SerdeError)?;

    match secrets.create(&PostParams::default(), &secret).await {
        Ok(_) => {
            info!(user_id, secret_name = %secret_name, "User secret created");
            Ok(secret_name)
        }
        Err(kube::Error::Api(ref e)) if e.code == 409 => {
            // Race condition: another provision created it first — update instead
            warn!(user_id, "User secret already exists (409), updating");
            update_user_secret(k8s, user_id, credentials).await?;
            Ok(secret_name)
        }
        Err(e) => Err(e),
    }
}

/// Update an existing user secret with fresh credentials
pub async fn update_user_secret(
    k8s: &K8sClient,
    user_id: &str,
    credentials: &UserCredentials,
) -> Result<(), kube::Error> {
    let secrets: Api<Secret> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let secret_name = generate_user_secret_name(user_id);

    let string_data = build_user_string_data(credentials);

    let mut labels = BTreeMap::new();
    labels.insert(
        "app.kubernetes.io/name".to_string(),
        "moatless-user-credentials".to_string(),
    );
    labels.insert(
        "app.kubernetes.io/managed-by".to_string(),
        "moatless".to_string(),
    );
    labels.insert("moatless.io/user-id".to_string(), user_id.to_string());

    let secret: Secret = serde_json::from_value(serde_json::json!({
        "apiVersion": "v1",
        "kind": "Secret",
        "metadata": {
            "name": secret_name,
            "namespace": k8s.namespace(),
            "labels": labels,
        },
        "type": "Opaque",
        "stringData": string_data,
    }))
    .map_err(kube::Error::SerdeError)?;

    secrets.replace(&secret_name, &PostParams::default(), &secret).await?;
    info!(user_id, secret_name = %secret_name, "User secret updated");

    Ok(())
}

/// Read credentials from an existing K8s Secret for a user.
///
/// Returns `None` if the secret does not exist.
pub async fn read_user_credentials(
    k8s: &K8sClient,
    user_id: &str,
) -> Option<UserCredentials> {
    let secrets: Api<Secret> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let secret_name = generate_user_secret_name(user_id);

    match secrets.get(&secret_name).await {
        Ok(secret) => {
            let data = secret.data.unwrap_or_default();
            let decode = |key: &str| -> Option<String> {
                data.get(key).and_then(|v| {
                    String::from_utf8(v.0.clone()).ok()
                })
            };

            let creds = UserCredentials {
                github_token: decode("GITHUB_TOKEN"),
                anthropic_api_key: decode("ANTHROPIC_API_KEY"),
                claude_oauth_token: decode("CLAUDE_CODE_OAUTH_TOKEN"),
                user_auth_token: decode("USER_AUTH_TOKEN"),
            };

            debug!(user_id, secret_name = %secret_name, "Read credentials from user secret");
            Some(creds)
        }
        Err(_) => {
            debug!(user_id, "No existing user secret found");
            None
        }
    }
}

/// Build stringData map from UserCredentials.
fn build_user_string_data(credentials: &UserCredentials) -> BTreeMap<String, String> {
    let mut string_data = BTreeMap::new();
    if let Some(ref token) = credentials.github_token {
        string_data.insert("GITHUB_TOKEN".to_string(), token.clone());
    }
    if let Some(ref key) = credentials.anthropic_api_key {
        string_data.insert("ANTHROPIC_API_KEY".to_string(), key.clone());
    }
    if let Some(ref token) = credentials.claude_oauth_token {
        string_data.insert("CLAUDE_CODE_OAUTH_TOKEN".to_string(), token.clone());
    }
    if let Some(ref token) = credentials.user_auth_token {
        string_data.insert("USER_AUTH_TOKEN".to_string(), token.clone());
    }
    string_data
}
