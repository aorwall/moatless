//! Granting access to a task's workspace `restic` repository.
//!
//! A presigned URL can't scope `rustic_core`'s prefix listing or carry the
//! repository's password, so access is an STS session naming only the prefixes
//! asked for, plus a derived per-task password; a fork's source prefix comes
//! along read-only. The sandbox takes this over gRPC; an admin, over HTTP with
//! the `restic` CLI.

use moatless_core::workspace_repository::derive_repository_password;
use moatless_sandbox_runtime::storage::workspace_repository_prefix;
use tracing::warn;

/// Everything one task's repository needs, and when the access stops working.
#[derive(Debug, Clone)]
pub struct RepositoryGrant {
    pub bucket: String,
    /// Key prefix the repository is rooted at, with no trailing slash.
    pub prefix: String,
    pub region: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: String,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    pub password: String,
    /// The fork source's repository, when the grant was asked for one.
    pub source: Option<SourceRepository>,
}

/// A fork source's repository: what the holder restores from and never writes.
///
/// Kept separate because the difference is the point: the session policy grants
/// this prefix `s3:GetObject` and listing only, encrypted under its own task's
/// password rather than the holder's.
#[derive(Debug, Clone)]
pub struct SourceRepository {
    /// Key prefix the source repository is rooted at, with no trailing slash.
    pub prefix: String,
    pub password: String,
}

impl RepositoryGrant {
    /// The repository as the `restic` CLI addresses it.
    pub fn restic_url(&self) -> String {
        format!(
            "s3:https://s3.{}.amazonaws.com/{}/{}",
            self.region, self.bucket, self.prefix
        )
    }
}

/// Why a grant could not be made.
///
/// One case: [`WorkspaceRepositoryAccess::new`] returns `None` without a role,
/// secret or bucket, so "not configured" is answered before any call to
/// [`WorkspaceRepositoryAccess::grant`].
#[derive(Debug, thiserror::Error)]
pub enum GrantError {
    #[error("assuming the workspace repository role failed")]
    Sts(#[source] moatless_s3::S3Error),
}

/// Mints per-task access to the workspace repositories in one bucket.
pub struct WorkspaceRepositoryAccess {
    sts: moatless_s3::RoleAssumer,
    role_arn: String,
    /// Every task's password is derived from this. Rotating it makes every
    /// existing repository unreadable.
    master_secret: String,
    bucket: String,
    region: String,
    workspace_prefix: String,
}

impl WorkspaceRepositoryAccess {
    /// `None` unless every piece is configured. A half-configured deployment
    /// has no repository to grant, and answering with a partial grant would
    /// fail later and further from the cause.
    pub fn new(
        sts: Option<moatless_s3::RoleAssumer>,
        role_arn: Option<String>,
        master_secret: Option<String>,
        bucket: Option<String>,
        region: String,
        workspace_prefix: String,
    ) -> Option<Self> {
        Some(Self {
            sts: sts?,
            role_arn: role_arn?,
            master_secret: master_secret?,
            bucket: bucket?,
            region,
            workspace_prefix,
        })
    }

    /// The prefix a grant for `task_id` is scoped to. The one place a task id
    /// becomes a key prefix here, so the invariant below has a call site to
    /// assert on rather than a helper two crates down.
    fn prefix_for(&self, task_id: &str) -> String {
        workspace_repository_prefix(&self.workspace_prefix, task_id)
    }

    /// Access to `task_id`'s repository, plus read-only access to
    /// `source_task_id`'s, and nothing else in the bucket.
    ///
    /// Only the task's prefix is writable — a fork could otherwise overwrite
    /// what its source restores from; without one, `None` reads nothing else.
    pub async fn grant(
        &self,
        task_id: &str,
        source_task_id: Option<&str>,
    ) -> Result<RepositoryGrant, GrantError> {
        let prefix = self.prefix_for(task_id);
        let source = source_task_id.map(|source_task_id| SourceRepository {
            prefix: self.prefix_for(source_task_id),
            password: derive_repository_password(&self.master_secret, source_task_id),
        });
        let session = moatless_s3::assume_role_for_prefix(
            &self.sts,
            &self.role_arn,
            &format!("moatless-workspace-{task_id}"),
            &self.bucket,
            &prefix,
            source.as_ref().map(|source| source.prefix.as_str()),
        )
        .await
        .map_err(|error| {
            warn!(%error, task_id, "Assuming the workspace repository role failed");
            GrantError::Sts(error)
        })?;

        Ok(RepositoryGrant {
            bucket: self.bucket.clone(),
            prefix,
            region: self.region.clone(),
            access_key_id: session.access_key_id,
            secret_access_key: session.secret_access_key,
            session_token: session.session_token,
            expires_at: session.expires_at,
            password: derive_repository_password(&self.master_secret, task_id),
            source,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Constructed, never called: every assertion here is about what a grant
    /// would be scoped to, not about reaching STS.
    fn sts_client() -> moatless_s3::RoleAssumer {
        moatless_s3::RoleAssumer::unconnected("eu-north-1")
    }

    #[test]
    fn a_half_configured_deployment_mints_nothing() {
        assert!(
            WorkspaceRepositoryAccess::new(
                None,
                Some("arn".into()),
                Some("secret".into()),
                Some("bucket".into()),
                "eu-north-1".into(),
                "workspaces".into(),
            )
            .is_none()
        );
    }

    /// A repository is written as well as read, so the writable prefix is the
    /// task's own even when a source is granted beside it: a fork writing its
    /// source's prefix would snapshot over the tree the source restores from.
    #[test]
    fn the_writable_prefix_is_the_task_s_own_and_never_its_fork_source() {
        let access = WorkspaceRepositoryAccess::new(
            Some(sts_client()),
            Some("arn:aws:iam::1:role/repo".into()),
            Some("secret".into()),
            Some("uploads".into()),
            "eu-north-1".into(),
            "workspaces".into(),
        )
        .expect("a fully configured deployment mints grants");

        assert_eq!(access.prefix_for("task-1"), "workspaces/task-1/repo");
        assert_ne!(
            access.prefix_for("fork-of-task-1"),
            access.prefix_for("task-1")
        );
    }

    #[test]
    fn the_restic_url_addresses_the_task_s_own_prefix() {
        let grant = RepositoryGrant {
            bucket: "uploads".into(),
            prefix: workspace_repository_prefix("workspaces", "task-1"),
            region: "eu-north-1".into(),
            access_key_id: String::new(),
            secret_access_key: String::new(),
            session_token: String::new(),
            expires_at: chrono::Utc::now(),
            password: String::new(),
            source: None,
        };
        assert_eq!(
            grant.restic_url(),
            "s3:https://s3.eu-north-1.amazonaws.com/uploads/workspaces/task-1/repo"
        );
    }

    /// The source repository is encrypted under its own task's password, so a
    /// grant that derived one password for both prefixes would hand the fork
    /// bytes it cannot decrypt.
    #[test]
    fn a_source_repository_carries_the_source_task_s_own_password() {
        let master = "secret";
        let source = SourceRepository {
            prefix: workspace_repository_prefix("workspaces", "task-1"),
            password: derive_repository_password(master, "task-1"),
        };
        assert_eq!(source.prefix, "workspaces/task-1/repo");
        assert_ne!(
            source.password,
            derive_repository_password(master, "fork-1")
        );
    }
}
