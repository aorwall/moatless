//! Deterministic target resolution from subdomain.
//!
//! Builds target URLs from task ID and port using Kubernetes service DNS.
//! No database query needed — the URL is computed from the subdomain.

use serde::{Deserialize, Serialize};

use crate::sandbox::k8s::naming::generate_service_name;

/// Target information for proxying
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetInfo {
    /// Full URL to proxy to
    pub target_url: String,
    /// Port on the target (for logging/metrics)
    pub port: u16,
}

/// Build the target URL deterministically from task ID and port.
///
/// Uses Kubernetes service DNS convention:
/// `http://moatless-ws-{task_id}.{namespace}.svc.cluster.local:{port}`
///
/// The service name is derived deterministically from the task ID (lowercase,
/// non-alphanumeric chars replaced with hyphens, truncated to 63 chars).
pub fn build_target(task_id: &str, port: u16, namespace: &str) -> TargetInfo {
    let service_name = generate_service_name(task_id);
    let target_url = format!(
        "http://{}.{}.svc.cluster.local:{}",
        service_name, namespace, port
    );
    TargetInfo { target_url, port }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_target() {
        let target = build_target("abc123", 3000, "workspaces");
        assert_eq!(
            target.target_url,
            "http://moatless-ws-abc123.workspaces.svc.cluster.local:3000"
        );
        assert_eq!(target.port, 3000);
    }

    #[test]
    fn test_build_target_different_ports() {
        let target = build_target("task-1", 8001, "sandbox");
        assert_eq!(
            target.target_url,
            "http://moatless-ws-task-1.sandbox.svc.cluster.local:8001"
        );

        let target = build_target("task-1", 4000, "sandbox");
        assert_eq!(
            target.target_url,
            "http://moatless-ws-task-1.sandbox.svc.cluster.local:4000"
        );
    }
}
