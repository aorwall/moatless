//! Kubernetes Secret delivery for server-targeted scoped secrets.

use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::Secret;
use kube::api::{Api, Patch, PatchParams};
use tracing::{debug, info, warn};

use super::client::K8sClient;
use super::naming::generate_secret_name;
use crate::sandbox::secrets::ServerSecretBundle;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServerSecretRefs {
    pub secret_name: String,
    pub by_server: BTreeMap<String, BTreeMap<String, String>>,
}

impl ServerSecretRefs {
    pub fn is_empty(&self) -> bool {
        self.by_server.is_empty()
    }

    pub fn get(&self, server_name: &str) -> Option<&BTreeMap<String, String>> {
        self.by_server.get(server_name)
    }
}

pub async fn create_task_server_secret(
    k8s: &K8sClient,
    task_id: &str,
    server_secrets: &ServerSecretBundle,
) -> Result<Option<ServerSecretRefs>, kube::Error> {
    if server_secrets.is_empty() {
        return Ok(None);
    }

    let secret_name = generate_secret_name(task_id);
    let mut string_data = BTreeMap::new();
    let mut by_server = BTreeMap::new();

    for (server_name, secrets) in server_secrets.iter() {
        let mut refs = BTreeMap::new();
        for (key, value) in secrets {
            let data_key = server_secret_data_key(server_name, key);
            string_data.insert(data_key.clone(), value.clone());
            refs.insert(key.clone(), data_key);
        }
        if !refs.is_empty() {
            by_server.insert(server_name.clone(), refs);
        }
    }

    if string_data.is_empty() {
        return Ok(None);
    }

    let secrets: Api<Secret> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let patch = serde_json::json!({
        "apiVersion": "v1",
        "kind": "Secret",
        "metadata": {
            "name": secret_name,
            "namespace": k8s.namespace(),
            "labels": {
                "app.kubernetes.io/name": "moatless-sandbox",
                "app.kubernetes.io/component": "server-secrets",
                "app.kubernetes.io/managed-by": "moatless",
                "moatless.io/task-id": task_id
            }
        },
        "type": "Opaque",
        "stringData": string_data
    });
    let params = PatchParams::apply("moatless-vibe").force();
    secrets
        .patch(&secret_name, &params, &Patch::Apply(&patch))
        .await?;

    info!(
        task_id,
        secret_name = %secret_name,
        server_count = by_server.len(),
        secret_count = server_secrets.secret_count(),
        "K8s server secret applied"
    );

    Ok(Some(ServerSecretRefs {
        secret_name,
        by_server,
    }))
}

pub async fn delete_task_server_secret(k8s: &K8sClient, task_id: &str) -> Result<(), kube::Error> {
    let secrets: Api<Secret> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let secret_name = generate_secret_name(task_id);

    match secrets.delete(&secret_name, &Default::default()).await {
        Ok(_) => {
            info!(task_id, secret_name = %secret_name, "K8s server secret deleted");
            Ok(())
        }
        Err(kube::Error::Api(ref e)) if e.code == 404 => {
            debug!(task_id, secret_name = %secret_name, "K8s server secret already deleted");
            Ok(())
        }
        Err(e) => {
            warn!(task_id, secret_name = %secret_name, error = %e, "Failed to delete K8s server secret");
            Err(e)
        }
    }
}

pub fn server_secret_data_key(server_name: &str, key: &str) -> String {
    let server = sanitize_secret_key_segment(server_name);
    let hash = stable_hash_hex(server_name);
    format!("{server}_{hash}_{key}")
}

fn sanitize_secret_key_segment(input: &str) -> String {
    let sanitized = input
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .take(80)
        .collect::<String>();
    if sanitized.is_empty() {
        "server".to_string()
    } else {
        sanitized
    }
}

fn stable_hash_hex(input: &str) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in input.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{:08x}", hash & 0xffff_ffff)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_secret_data_key_is_stable_and_sanitized() {
        let key = server_secret_data_key("Preview Server", "STRIPE_SECRET_KEY");
        assert!(key.starts_with("Preview_Server_"));
        assert!(key.ends_with("_STRIPE_SECRET_KEY"));
        assert_eq!(
            key,
            server_secret_data_key("Preview Server", "STRIPE_SECRET_KEY")
        );
    }
}
