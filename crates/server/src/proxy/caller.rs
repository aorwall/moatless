//! The sandbox a presented task token speaks for.

use std::sync::Arc;

use moatless_core::sandbox_jwt::{
    GenerationError, TokenGenerations, check_generation, verify_user_auth_token,
};
use moatless_types::attribution::Attribution;
use tracing::warn;

use crate::sandbox::lifecycle::SandboxIngress;

/// The sandbox a verified task token was minted for, and the identities a
/// credential for it resolves against.
#[derive(Debug, Clone)]
pub struct SandboxScope {
    pub sandbox: String,
    /// The token's own `userId`, the Sandbox's driver when it was minted.
    pub token_user: String,
    /// The Sandbox's driver, else its owner, which a run-as identity
    /// overrides.
    pub user: String,
    pub workspace: String,
    /// The run-as identity the Sandbox row pins, ahead of the Workspace's.
    pub run_as: Option<String>,
    /// The service account the Sandbox row records for its pod, which a
    /// service-proxy rule's `serviceAccounts` matches. `None` for the default.
    pub service_account: Option<String>,
    /// Who a write from the Sandbox credits.
    pub attribution: Attribution,
    /// The Task a write from the Sandbox is recorded for: the token's
    /// `taskId`, else the founding Task, whose id the Sandbox takes, even when
    /// a Member Task made the write.
    pub task: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum CallerError {
    /// Every rejection of the caller's own task token.
    #[error("{0}")]
    Unauthorized(String),
    /// The token verified, but its Sandbox row is gone.
    #[error("The task token names a sandbox that no longer exists")]
    SandboxGone,
    /// A store failed; the caller's request may succeed later.
    #[error("{0}")]
    Upstream(String),
}

/// Verifies task tokens for every proxy a pod reaches the backend through.
pub struct SandboxCallers {
    sandboxes: Arc<SandboxIngress>,
    token_generations: Arc<dyn TokenGenerations>,
    /// `None` in a deployment that mints no task tokens, which refuses every
    /// proxied request rather than admitting an unverifiable one.
    task_token_secret: Option<String>,
}

impl SandboxCallers {
    pub(crate) fn new(
        sandboxes: Arc<SandboxIngress>,
        token_generations: Arc<dyn TokenGenerations>,
        task_token_secret: Option<String>,
    ) -> Self {
        Self {
            sandboxes,
            token_generations,
            task_token_secret,
        }
    }

    /// Verify `task_token`, at its Sandbox's current generation, and read the
    /// Sandbox it was minted for.
    pub async fn authenticate(
        &self,
        task_token: Option<&str>,
    ) -> Result<SandboxScope, CallerError> {
        let task_token = task_token
            .ok_or_else(|| CallerError::Unauthorized("No task token was presented".to_string()))?;
        let secret = self.task_token_secret.as_deref().ok_or_else(|| {
            CallerError::Unauthorized(
                "This deployment verifies no task tokens, so it proxies no sandbox traffic"
                    .to_string(),
            )
        })?;
        let claims = verify_user_auth_token(task_token, secret)
            .map_err(|e| CallerError::Unauthorized(format!("Invalid task token: {e}")))?;
        // `generate_user_auth_token` binds the token to a Sandbox, not to a
        // Task: a shared Sandbox runs several Member tasks off one pod.
        let sandbox_id = claims.sandbox_id.clone().ok_or_else(|| {
            CallerError::Unauthorized("The task token names no sandbox".to_string())
        })?;
        check_generation(
            self.token_generations.as_ref(),
            &sandbox_id,
            claims.generation,
        )
        .await
        .map_err(|e| match e {
            GenerationError::Stale { .. } | GenerationError::NoSandbox => {
                CallerError::Unauthorized(e.to_string())
            }
            GenerationError::Lookup(_) => CallerError::Upstream(e.to_string()),
        })?;

        let workspace_id = self
            .sandboxes
            .workspace_id(&sandbox_id)
            .await
            .map_err(|e| {
                warn!(error = %e, sandbox_id, "Failed to read the sandbox for a proxied request");
                CallerError::Upstream("Failed to read the sandbox".to_string())
            })?
            .ok_or(CallerError::SandboxGone)?;
        // Fails closed rather than degrading to the token's user: that is the
        // driver only while the read agrees.
        let identity = self
            .sandboxes
            .identity(&sandbox_id)
            .await
            .map_err(|e| {
                warn!(error = %e, sandbox_id, "Failed to read the sandbox identity");
                CallerError::Upstream("Failed to read the sandbox".to_string())
            })?
            .unwrap_or_default();
        Ok(SandboxScope {
            attribution: identity.attribution(),
            user: identity
                .acting_user_id
                .unwrap_or_else(|| claims.user_id.clone()),
            token_user: claims.user_id,
            task: recorded_task(claims.task_id, &sandbox_id),
            sandbox: sandbox_id,
            workspace: workspace_id,
            run_as: identity.pinned_run_as,
            service_account: identity.service_account,
        })
    }
}

/// [`SandboxScope::task`] for a token naming `task_id` and `sandbox_id`.
///
/// `generate_user_auth_token` mints no `taskId`: without the fallback neither
/// proxy records what a Task made, and every push and comment it makes is
/// delivered back to it as someone else's.
fn recorded_task(task_id: Option<String>, sandbox_id: &str) -> Option<String> {
    task_id.or_else(|| Some(sandbox_id.to_string()))
}

#[cfg(test)]
mod tests {
    use moatless_core::sandbox_jwt::generate_user_auth_token;

    use super::*;

    #[test]
    fn a_minted_task_token_records_writes_for_the_founding_task() {
        let token = generate_user_auth_token("user-1", "user", "task-1", 0, "secret", 600).unwrap();
        let claims = verify_user_auth_token(&token, "secret").unwrap();
        let sandbox_id = claims.sandbox_id.unwrap();
        assert_eq!(
            recorded_task(claims.task_id, &sandbox_id).as_deref(),
            Some("task-1")
        );
        assert_eq!(
            recorded_task(Some("member-1".into()), &sandbox_id).as_deref(),
            Some("member-1")
        );
    }
}
