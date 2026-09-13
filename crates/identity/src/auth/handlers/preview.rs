//! The preview-host gate: a `task-id--slug` hostname is reachable only by a
//! scope that may see that task.

use crate::auth::error::AuthError;
use moatless_types::access_control::ScopeContext;

/// The preview-URL session gate: when `forwarded_host` names a preview host,
/// `Forbidden` unless `scope_ctx` may reach its task.
///
/// A non-preview host (or none) passes without consulting the port.
pub(super) async fn enforce_preview_access(
    task_access: &dyn moatless_core::http_auth::TaskAccess,
    scope_ctx: &ScopeContext,
    forwarded_host: Option<&str>,
) -> Result<(), AuthError> {
    let Some(host) = forwarded_host else {
        return Ok(());
    };
    let Some(task_id) = extract_task_id_from_hostname(host) else {
        return Ok(());
    };
    if task_access.may_access_task(task_id, scope_ctx).await {
        Ok(())
    } else {
        Err(AuthError::Forbidden)
    }
}

/// Extract task ID from a preview hostname.
///
/// Format: `{task_id}--{port}.{proxy_domain}`
fn extract_task_id_from_hostname(host: &str) -> Option<&str> {
    let first_segment = host.split('.').next()?;
    if !first_segment.contains("--") {
        return None;
    }
    let task_id = first_segment.split("--").next()?;
    if task_id.is_empty() {
        None
    } else {
        Some(task_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A `TaskAccess` that answers with a fixed verdict and counts its calls,
    /// so a test can assert both the returned decision and whether the gate
    /// consulted the port at all.
    struct FakeTaskAccess {
        allow: bool,
        calls: AtomicUsize,
    }

    impl FakeTaskAccess {
        fn new(allow: bool) -> Self {
            Self {
                allow,
                calls: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait::async_trait]
    impl moatless_core::http_auth::TaskAccess for FakeTaskAccess {
        async fn may_access_task(&self, _task_id: &str, _ctx: &ScopeContext) -> bool {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.allow
        }
    }

    fn scope_ctx() -> ScopeContext {
        ScopeContext {
            user_id: "user-1".to_string(),
            is_admin: false,
        }
    }

    #[tokio::test]
    async fn preview_gate_denies_task_the_scope_cannot_see() {
        let access = FakeTaskAccess::new(false);
        let result = enforce_preview_access(
            &access,
            &scope_ctx(),
            Some("task-1--abc.preview.example.com"),
        )
        .await;
        assert!(matches!(result, Err(AuthError::Forbidden)));
        assert_eq!(access.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn preview_gate_allows_task_the_scope_can_see() {
        let access = FakeTaskAccess::new(true);
        let result = enforce_preview_access(
            &access,
            &scope_ctx(),
            Some("task-1--abc.preview.example.com"),
        )
        .await;
        assert!(result.is_ok());
        assert_eq!(access.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn non_preview_host_passes_without_consulting_the_port() {
        // A host with no `--` segment is not a preview URL; the gate must not
        // deny it and must not touch the port.
        let access = FakeTaskAccess::new(false);
        let result = enforce_preview_access(&access, &scope_ctx(), Some("app.example.com")).await;
        assert!(result.is_ok());
        assert_eq!(access.calls.load(Ordering::SeqCst), 0);
    }
}
