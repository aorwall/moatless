//! Kubernetes client wrapper for workspace operations.

use kube::Client;
use std::sync::Arc;
use tracing::info;

/// A failed Kubernetes API call, keeping the client's error as its source.
#[derive(Debug, thiserror::Error)]
#[error("{context}: {source}")]
pub struct K8sError {
    context: &'static str,
    #[source]
    source: kube::Error,
}

impl K8sError {
    pub(crate) fn new(context: &'static str, source: kube::Error) -> Self {
        Self { context, source }
    }
}

/// Shared K8s client state
#[derive(Clone)]
pub struct K8sClient {
    pub(crate) client: Client,
    pub namespace: Arc<String>,
}

impl K8sClient {
    /// Create a new K8s client using in-cluster or default config
    pub async fn new(namespace: String) -> Result<Self, K8sError> {
        let client = Client::try_default()
            .await
            .map_err(|e| K8sError::new("connecting to Kubernetes", e))?;
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
