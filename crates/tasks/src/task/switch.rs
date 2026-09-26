//! Switching a Task's Agent profile, and with it possibly its harness.
//!
//! Only the rows change here: the next delivery carries the new profile, and
//! the pod rebuilds its harness and transcodes the history into it.

use std::sync::Arc;

use moatless_ports::identity::{HarnessCredentials, SecretCatalog};
use moatless_types::access_control::{ScopeContext, ScopedResource};
use moatless_types::agent::{AgentProfile, Harness};

use super::dao::TaskDao;
use super::error::TaskError;
use super::switch_dao;
use crate::agent::catalog::{self, harness_of, use_tmux_for};
use crate::credential_sync::CredentialSync;
use crate::message::dao::MessageDao;
use crate::trajectory::operations::Trajectories;

pub(crate) const FROZEN_SWITCH: &str =
    "This Task was created before recording and cannot switch harness";

const CODEX_MISSING: &str = "Your Codex connection is missing or has expired. Reconnect Codex in Settings before switching to Codex.";

/// What the caller holds credentials through, read to refuse a profile it
/// cannot run.
pub struct SwitchCredentials<'a> {
    pub secrets: &'a dyn SecretCatalog,
    pub provider_tokens: &'a dyn HarnessCredentials,
    pub credential_sync: &'a CredentialSync,
}

/// The result of a switch request that was not refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SwitchOutcome {
    /// The Task already runs this profile; nothing was written.
    Unchanged,
    Switched {
        from: AgentProfile,
    },
}

/// Setting the Agent profile a Task's next turn runs.
pub struct TaskSwitchService {
    db: moatless_core::db::DbPool,
    tasks: Arc<dyn TaskDao>,
    messages: Arc<dyn MessageDao>,
    trajectories: Arc<Trajectories>,
}

impl TaskSwitchService {
    pub(crate) fn new(
        db: moatless_core::db::DbPool,
        tasks: Arc<dyn TaskDao>,
        messages: Arc<dyn MessageDao>,
        trajectories: Arc<Trajectories>,
    ) -> Self {
        Self {
            db,
            tasks,
            messages,
            trajectories,
        }
    }

    /// The profile a Task switching to `harness` runs: the first catalog entry
    /// of it, and for Claude Code the one whose transport `sandbox_id`'s pod
    /// runs, so a harness picked by name never answers a transport 409.
    pub async fn profile_for(
        &self,
        sandbox_id: Option<&str>,
        harness: Harness,
    ) -> Result<&'static str, TaskError> {
        let sandbox_tmux = match sandbox_id {
            Some(id) => switch_dao::sandbox_tmux(&self.db, id).await?,
            None => None,
        };
        let entry = catalog::catalog()
            .iter()
            .filter(|e| e.harness == harness)
            .find(|e| {
                harness != Harness::ClaudeCode || e.params.use_tmux == sandbox_tmux.unwrap_or(false)
            })
            .or_else(|| catalog::catalog().iter().find(|e| e.harness == harness));
        entry
            .map(|e| e.id)
            .ok_or_else(|| TaskError::BadRequest(format!("no agentProfile runs {harness}")))
    }

    /// Set `profile` (and `model`, else no model) as what `task_id`'s next
    /// turn runs. A running turn finishes on the profile it started with.
    /// `Unchanged` for the current profile, whose `model` the caller writes.
    pub async fn switch_profile(
        &self,
        scope_ctx: &ScopeContext,
        task_id: &str,
        profile: &str,
        model: Option<&str>,
        credentials: SwitchCredentials<'_>,
    ) -> Result<SwitchOutcome, TaskError> {
        let task = self
            .tasks
            .find_by_id(task_id, scope_ctx)
            .await?
            .ok_or_else(|| TaskError::NotFound(task_id.to_string()))?;
        if !scope_ctx.can_control(
            &ScopedResource::borrowed(
                task.scope,
                task.owner_user_id.as_deref().or(task.created_by.as_deref()),
            ),
            task.visibility,
        ) {
            return Err(TaskError::NotFound(task_id.to_string()));
        }
        catalog::validate_agent_model(profile, model).map_err(TaskError::BadRequest)?;
        if task.agent_profile.as_str() == profile {
            return Ok(SwitchOutcome::Unchanged);
        }
        let Some(entry) = catalog::find(profile) else {
            unreachable!("validate_agent_model accepted {profile}");
        };

        if !crate::agent::service::is_installed(
            credentials.secrets,
            credentials.provider_tokens,
            scope_ctx,
            entry,
        )
        .await
        {
            return Err(TaskError::BadRequest(
                entry.missing_credential_hint.to_string(),
            ));
        }
        // A Codex secret needs no refresh; only a ChatGPT connection can have
        // expired while still counting as installed.
        if profile == AgentProfile::CODEX
            && !holds_a_key(credentials.secrets, scope_ctx, entry).await
        {
            if let Err(moatless_identity::credentials::CredentialError::NotFound(_)) = credentials
                .credential_sync
                .ensure_fresh_codex_for(&scope_ctx.user_id, None)
                .await
            {
                return Err(TaskError::BadRequest(CODEX_MISSING.to_string()));
            }
        }

        let harness_changes = harness_of(task.agent_profile.as_str()) != Some(entry.harness);
        if harness_changes
            && self.messages.fork_anchors(task_id, None).await?.is_some()
            && self.trajectories.head(task_id).await?.is_none()
        {
            return Err(TaskError::Conflict(FROZEN_SWITCH.to_string()));
        }

        let sandbox_id = task.sandbox_id.as_deref().unwrap_or(task_id);
        let sandbox_tmux = switch_dao::sandbox_tmux(&self.db, sandbox_id).await?;
        if entry.harness == Harness::ClaudeCode
            && sandbox_tmux.is_some_and(|tmux| tmux != use_tmux_for(profile))
        {
            return Err(TaskError::Conflict(format!(
                "This Sandbox cannot run {}",
                entry.label
            )));
        }
        switch_dao::write_profile(&self.db, task_id, profile, model).await?;

        tracing::info!(
            task_id,
            from = %task.agent_profile,
            to = profile,
            harness_changes,
            "Task agent profile switched"
        );
        Ok(SwitchOutcome::Switched {
            from: task.agent_profile,
        })
    }
}

/// Whether `ctx` sees one of `entry`'s credential secrets.
async fn holds_a_key(
    secrets: &dyn SecretCatalog,
    ctx: &ScopeContext,
    entry: &catalog::AgentProfileEntry,
) -> bool {
    secrets.list(ctx).await.is_ok_and(|keys| {
        keys.iter()
            .any(|key| entry.credential_keys.contains(&key.as_str()))
    })
}
