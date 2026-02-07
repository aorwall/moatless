//! Kubernetes client wrapper for workspace operations.

use kube::Client;
use std::sync::Arc;
use tracing::info;

/// Shared K8s client state
#[derive(Clone)]
pub struct K8sClient {
    pub client: Client,
    pub namespace: Arc<String>,
}

impl K8sClient {
    /// Create a new K8s client using in-cluster or default config
    pub async fn new(namespace: String) -> Result<Self, kube::Error> {
        let client = Client::try_default().await?;
        info!(namespace = %namespace, "Kubernetes client initialized");
        Ok(Self {
            client,
            namespace: Arc::new(namespace),
        })
    }

    pub fn namespace(&self) -> &str {
        &self.namespace
    }
}
