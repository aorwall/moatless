//! The `secret_audit_events` trail: who reached for which credential, and how
//! it went.

use std::sync::Arc;

use async_trait::async_trait;
use tracing::warn;

use super::models::SecretRecord;

/// `secret_audit_events.scope_type` for a workspace credential. The table is
/// shared with `secrets`, so the scope value is what tells the two kinds of row
/// apart.
const WORKSPACE_SCOPE: &str = "workspace";

pub(crate) struct SecretAuditInput<'a> {
    pub secret_id: Option<&'a str>,
    pub actor_id: &'a str,
    pub action: &'a str,
    pub scope_type: &'a str,
    pub scope_id: Option<&'a str>,
    pub key: &'a str,
    pub result: &'a str,
}

#[async_trait]
pub(crate) trait SecretAuditDao: Send + Sync {
    async fn record(&self, input: SecretAuditInput<'_>) -> Result<(), sqlx::Error>;
}

/// What a caller did to a workspace credential.
#[derive(Debug, Clone, Copy)]
pub enum CredentialAction {
    Set,
    Delete,
}

impl CredentialAction {
    fn as_str(self) -> &'static str {
        match self {
            Self::Set => "set",
            Self::Delete => "delete",
        }
    }
}

/// How the attempt ended.
#[derive(Debug, Clone, Copy)]
pub enum AuditOutcome {
    Success,
    /// The access-control check rejected the attempt, which is exactly the
    /// event the trail exists for.
    Denied,
}

impl AuditOutcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Denied => "denied",
        }
    }
}

/// The one way to append to `secret_audit_events`.
pub struct SecretAudit {
    events: Arc<dyn SecretAuditDao>,
}

impl SecretAudit {
    pub(crate) fn new(events: Arc<dyn SecretAuditDao>) -> Self {
        Self { events }
    }

    /// Record a reach for a credential pinned to a workspace.
    ///
    /// Best-effort: the described operation already committed, so a failed
    /// write is logged and dropped rather than propagated.
    pub async fn record_workspace_credential(
        &self,
        actor_id: &str,
        action: CredentialAction,
        workspace_id: &str,
        key: &str,
        outcome: AuditOutcome,
    ) {
        if let Err(e) = self
            .events
            .record(SecretAuditInput {
                secret_id: None,
                actor_id,
                action: action.as_str(),
                scope_type: WORKSPACE_SCOPE,
                scope_id: Some(workspace_id),
                key,
                result: outcome.as_str(),
            })
            .await
        {
            warn!(
                error = %e,
                workspace_id,
                key,
                action = action.as_str(),
                "Failed to record workspace credential audit event"
            );
        }
    }

    /// Record a reach for a stored secret. Fail-closed: the caller propagates
    /// the error, so an unrecordable read does not return a value.
    pub(crate) async fn record_secret(
        &self,
        secret_id: Option<&str>,
        actor_id: &str,
        action: &str,
        record: &SecretRecord,
        result: &str,
    ) -> Result<(), sqlx::Error> {
        // `secrets.scope` goes in `scope_type` and `secrets.owner_user_id` in
        // `scope_id`, which is the shape the table was written with.
        self.events
            .record(SecretAuditInput {
                secret_id,
                actor_id,
                action,
                scope_type: &record.scope,
                scope_id: record.owner_user_id.as_deref(),
                key: &record.key,
                result,
            })
            .await
    }
}
