//! Granting access to a task's workspace `restic` repository.
//!
//! Two things a presigned URL cannot carry. `rustic_core` lists the repository
//! by prefix on every open, and a signature covers one key; and the repository
//! is encrypted, so reaching the bytes without the password reads nothing.
//!
//! Both are scoped to one task: the STS session policy names only that task's
//! prefix, and the password is derived from the task id. The sandbox takes this
//! over gRPC to run its own sync; an administrator takes it over HTTP to open
//! the repository with the `restic` CLI.

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
/// One case, because [`WorkspaceRepositoryAccess::new`] returns `None` on a
/// deployment with no role, secret or bucket — so a value of that type is
/// always fully configured and the "not configured" answer is the `Option`,
/// upstream of every call to [`WorkspaceRepositoryAccess::grant`].
#[derive(Debug, thiserror::Error)]
pub enum GrantError {
    #[error("assuming the workspace repository role failed")]
    Sts(#[source] moatless_s3::S3Error),
}

/// Mints per-task access to the workspace repositories in one bucket.
pub struct WorkspaceRepositoryAccess {
    sts: aws_sdk_sts::Client,
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
        sts: Option<aws_sdk_sts::Client>,
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

    /// Access to `task_id`'s repository, and nothing else in the bucket.
    ///
    /// The task's own prefix, never a fork's source. A restore reads a parent's
    /// tar and mirror, but a repository is written as well as read, so pointing
    /// a fork at its parent would let it snapshot over the tree the parent
    /// restores from.
    pub async fn grant(&self, task_id: &str) -> Result<RepositoryGrant, GrantError> {
        let prefix = self.prefix_for(task_id);
        let session = moatless_s3::assume_role_for_prefix(
            &self.sts,
            &self.role_arn,
            &format!("moatless-workspace-{task_id}"),
            &self.bucket,
            &prefix,
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
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Constructed, never called: every assertion here is about what a grant
    /// would be scoped to, not about reaching STS.
    fn sts_client() -> aws_sdk_sts::Client {
        aws_sdk_sts::Client::from_conf(
            aws_sdk_sts::Config::builder()
                .behavior_version_latest()
                .region(aws_sdk_sts::config::Region::new("eu-north-1"))
                .build(),
        )
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

    /// A fork restores from its parent's tar and mirror, but a repository is
    /// written as well as read: pointing a fork at its parent's prefix would
    /// let it snapshot over the tree the parent restores from.
    #[test]
    fn the_granted_prefix_is_the_task_s_own_and_never_a_fork_source() {
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
        };
        assert_eq!(
            grant.restic_url(),
            "s3:https://s3.eu-north-1.amazonaws.com/uploads/workspaces/task-1/repo"
        );
    }
}
