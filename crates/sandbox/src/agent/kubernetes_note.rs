//! The system-prompt note warning that `kubectl` has no credentials.

/// The token `docker/sandbox-kubeconfig` points `kubectl` at.
pub(crate) const SERVICE_ACCOUNT_TOKEN: &str =
    "/var/run/secrets/kubernetes.io/serviceaccount/token";

/// The note for a pod in Kubernetes with no service-account token mounted.
///
/// A token is mounted only when the workspace or the sandbox's role identity
/// (`SANDBOX_ROLE_IDENTITIES_JSON`) names a service account. Without this note
/// a skill that assumes one spends the turn on port-forwards and preview setup
/// that cannot authenticate.
pub(crate) fn build(kubectl_unauthenticated: bool) -> Option<String> {
    kubectl_unauthenticated.then(|| {
        "## Kubernetes\n\
         This sandbox has no Kubernetes credentials: neither its workspace \
         nor its user's role grants a service account, so no token is \
         mounted and every `kubectl` call fails. A step that needs the \
         cluster — a port-forward, a preview environment, an ArgoCD check, \
         a secret read — cannot run here; say so in your result rather than \
         retrying, and verify another way where one exists."
            .to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_pod_without_credentials_is_warned() {
        assert_eq!(build(false), None);
        assert!(build(true).is_some_and(|note| note.contains("kubectl")));
    }
}
