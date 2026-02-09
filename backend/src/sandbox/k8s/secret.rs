//! Kubernetes Secret lifecycle operations for task credentials.

use k8s_openapi::api::core::v1::Secret;
use kube::api::{Api, PostParams};
use std::collections::BTreeMap;
use tracing::{debug, info, warn};

use super::client::K8sClient;
use super::naming::generate_secret_name;
use crate::sandbox::types::TaskCredentials;

/// Find existing secret for a task
pub async fn find_task_secret(k8s: &K8sClient, task_id: &str) -> Option<String> {
    let secrets: Api<Secret> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let secret_name = generate_secret_name(task_id);

    match secrets.get(&secret_name).await {
        Ok(_) => {
            debug!(task_id, secret_name = %secret_name, "Found existing secret");
            Some(secret_name)
        }
        Err(_) => None,
    }
}

/// Create a Kubernetes Secret for task credentials
pub async fn create_task_secret(
    k8s: &K8sClient,
    task_id: &str,
    credentials: &TaskCredentials,
) -> Result<String, kube::Error> {
    let secrets: Api<Secret> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let secret_name = generate_secret_name(task_id);

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
    if let Some(ref token) = credentials.task_auth_token {
        string_data.insert("TASK_AUTH_TOKEN".to_string(), token.clone());
    }

    let mut labels = BTreeMap::new();
    labels.insert(
        "app.kubernetes.io/name".to_string(),
        "moatless-task-secrets".to_string(),
    );
    labels.insert(
        "app.kubernetes.io/managed-by".to_string(),
        "moatless".to_string(),
    );
    labels.insert("moatless.io/task-id".to_string(), task_id.to_string());

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

    secrets.create(&PostParams::default(), &secret).await?;
    info!(task_id, secret_name = %secret_name, "Task secret created");

    Ok(secret_name)
}

/// Update an existing secret with fresh credentials
pub async fn update_task_secret(
    k8s: &K8sClient,
    task_id: &str,
    credentials: &TaskCredentials,
) -> Result<(), kube::Error> {
    let secrets: Api<Secret> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let secret_name = generate_secret_name(task_id);

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
    if let Some(ref token) = credentials.task_auth_token {
        string_data.insert("TASK_AUTH_TOKEN".to_string(), token.clone());
    }

    let mut labels = BTreeMap::new();
    labels.insert(
        "app.kubernetes.io/name".to_string(),
        "moatless-task-secrets".to_string(),
    );
    labels.insert(
        "app.kubernetes.io/managed-by".to_string(),
        "moatless".to_string(),
    );
    labels.insert("moatless.io/task-id".to_string(), task_id.to_string());

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
    info!(task_id, secret_name = %secret_name, "Task secret updated");

    Ok(())
}

/// Read credentials from an existing K8s Secret for a task.
///
/// Returns `None` if the secret does not exist.
/// This is used by auto-provisioning to recover credentials from a previous provision.
pub async fn read_task_credentials(
    k8s: &K8sClient,
    task_id: &str,
) -> Option<TaskCredentials> {
    let secrets: Api<Secret> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let secret_name = generate_secret_name(task_id);

    match secrets.get(&secret_name).await {
        Ok(secret) => {
            // stringData is write-only in the K8s API; read from `data` (base64-encoded)
            let data = secret.data.unwrap_or_default();
            let decode = |key: &str| -> Option<String> {
                data.get(key).and_then(|v| {
                    String::from_utf8(v.0.clone()).ok()
                })
            };

            let creds = TaskCredentials {
                github_token: decode("GITHUB_TOKEN"),
                anthropic_api_key: decode("ANTHROPIC_API_KEY"),
                claude_oauth_token: decode("CLAUDE_CODE_OAUTH_TOKEN"),
                task_auth_token: decode("TASK_AUTH_TOKEN"),
            };

            debug!(task_id, secret_name = %secret_name, "Read credentials from existing secret");
            Some(creds)
        }
        Err(_) => {
            debug!(task_id, "No existing secret found for credential recovery");
            None
        }
    }
}

/// Delete secret for a task
pub async fn delete_task_secret(k8s: &K8sClient, task_id: &str) -> Result<(), kube::Error> {
    let secrets: Api<Secret> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let secret_name = generate_secret_name(task_id);

    match secrets.delete(&secret_name, &Default::default()).await {
        Ok(_) => {
            info!(task_id, secret_name = %secret_name, "Task secret deleted");
            Ok(())
        }
        Err(kube::Error::Api(ref e)) if e.code == 404 => {
            debug!(task_id, "Task secret not found, already deleted");
            Ok(())
        }
        Err(e) => {
            warn!(task_id, error = %e, "Failed to delete task secret");
            Err(e)
        }
    }
}
