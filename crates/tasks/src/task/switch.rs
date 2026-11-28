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
use super::models::Task;
use super::switch_dao;
use crate::agent::catalog::{self, use_tmux_for};
use crate::credential_sync::CredentialSync;
use crate::message::dao::MessageDao;
use crate::task_event::models::InsertTaskEvent;
use crate::trajectory::operations::Trajectories;
use moatless_types::event::EventType;
use moatless_types::task::TaskSubType;

pub(crate) const FROZEN_SWITCH: &str =
    "This Task was created before recording and cannot switch harness";

const CODEX_MISSING: &str = "Your Codex connection is missing or has expired. Reconnect Codex in Settings before switching to Codex.";

/// What the caller holds credentials through, read to refuse a profile it
/// cannot run.
pub struct SwitchCredentials<'a> {
    pub hub_available: bool,
    pub secrets: &'a dyn SecretCatalog,
    pub provider_tokens: &'a dyn HarnessCredentials,
    pub credential_sync: &'a CredentialSync,
}

/// Why the platform switched a Task, shown in its conversation.
pub struct SwitchNote<'a> {
    /// The sentence the conversation shows, e.g. "Switched from Codex to
    /// Claude Code: Codex hit its usage limit".
    pub summary: &'a str,
    /// The turn that prompted the switch.
    pub turn_number: Option<i32>,
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
    ) -> Result<AgentProfile, TaskError> {
        let sandbox_tmux = match sandbox_id {
            Some(id) => switch_dao::sandbox_tmux(&self.db, id).await?,
            None => None,
        };
        let entry = catalog::catalog()
            .filter(|e| e.harness() == harness)
            .find(|e| {
                harness != Harness::ClaudeCode || e.params.use_tmux == sandbox_tmux.unwrap_or(false)
            })
            .or_else(|| catalog::catalog().find(|e| e.harness() == harness));
        entry
            .map(|e| e.profile)
            .ok_or_else(|| TaskError::BadRequest(format!("no agentProfile runs {harness}")))
    }

    /// Set `profile` (and `model`, else no model) as what `task_id`'s next
    /// turn runs. A running turn finishes on the profile it started with.
    /// `Unchanged` for the current profile, whose `model` the caller writes.
    pub async fn switch_profile(
        &self,
        scope_ctx: &ScopeContext,
        task_id: &str,
        profile: AgentProfile,
        model: Option<&str>,
        credentials: SwitchCredentials<'_>,
    ) -> Result<SwitchOutcome, TaskError> {
        let task = self
            .tasks
            .find_by_id(task_id, scope_ctx)
            .await?
            .ok_or_else(|| TaskError::NotFound(task_id.to_string()))?;
        if !scope_ctx.can_write(
            &ScopedResource::task(task.owner_user_id.as_deref().or(task.created_by.as_deref())),
            Some(task.visibility),
        ) {
            return Err(TaskError::NotFound(task_id.to_string()));
        }
        self.switch_task(task, profile, model, scope_ctx, credentials, None)
            .await
    }

    /// Switch a Loop's Task to `profile` and `model` (else its default model),
    /// after its provider refused a turn, and record `note` in its conversation. The
    /// platform acts here, so no caller's write is checked; the credentials
    /// checked are `run_as`'s, the account the Task runs as.
    pub async fn switch_for_fallback(
        &self,
        task_id: &str,
        profile: AgentProfile,
        model: Option<&str>,
        run_as: &ScopeContext,
        credentials: SwitchCredentials<'_>,
        note: &SwitchNote<'_>,
    ) -> Result<SwitchOutcome, TaskError> {
        #[expect(
            clippy::disallowed_methods,
            reason = "an agent fallback has no caller; `run_as` scopes credentials"
        )]
        let system = ScopeContext::system();
        let task = self
            .tasks
            .find_by_id(task_id, &system)
            .await?
            .ok_or_else(|| TaskError::NotFound(task_id.to_string()))?;
        self.switch_task(task, profile, model, run_as, credentials, Some(note))
            .await
    }

    async fn switch_task(
        &self,
        task: Task,
        profile: AgentProfile,
        model: Option<&str>,
        scope_ctx: &ScopeContext,
        credentials: SwitchCredentials<'_>,
        note: Option<&SwitchNote<'_>>,
    ) -> Result<SwitchOutcome, TaskError> {
        let task_id = task.task_id.as_str();
        catalog::validate_agent_model(profile, model).map_err(TaskError::BadRequest)?;
        if task.agent_profile == profile {
            return Ok(SwitchOutcome::Unchanged);
        }
        let entry = catalog::entry(profile);

        if !credentials.hub_available
            && !crate::agent::service::is_installed(
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
        if profile == AgentProfile::Codex
            && !credentials.hub_available
            && !crate::agent::service::holds_a_key(credentials.secrets, scope_ctx, entry).await
        {
            if let Err(moatless_identity::credentials::CredentialError::NotFound(_)) = credentials
                .credential_sync
                .ensure_fresh_codex_for(&scope_ctx.user_id)
                .await
            {
                return Err(TaskError::BadRequest(CODEX_MISSING.to_string()));
            }
        }

        let harness_changes = task.agent_profile.harness() != profile.harness();
        if harness_changes
            && self.trajectories.head(task_id).await?.is_none()
            && self.messages.is_frozen(task_id).await?
        {
            return Err(TaskError::Conflict(FROZEN_SWITCH.to_string()));
        }

        let sandbox_id = task.sandbox_id.as_deref().unwrap_or(task_id);
        let sandbox_tmux = switch_dao::sandbox_tmux(&self.db, sandbox_id).await?;
        if profile.harness() == Harness::ClaudeCode
            && sandbox_tmux.is_some_and(|tmux| tmux != use_tmux_for(profile))
        {
            return Err(TaskError::Conflict(format!(
                "This Sandbox cannot run {}",
                entry.label
            )));
        }
        // A person's switch gets a note too: every switch is a handoff row in
        // the conversation.
        let chosen = format!(
            "Switched from {} to {}",
            catalog::entry(task.agent_profile).label,
            entry.label
        );
        let chosen = SwitchNote {
            summary: &chosen,
            turn_number: None,
        };
        let event = switch_event(
            task_id,
            task.agent_profile,
            profile,
            note.unwrap_or(&chosen),
            note.is_none(),
        );
        switch_dao::write_profile(&self.db, task_id, profile, model, Some(&event)).await?;

        tracing::info!(
            task_id,
            from = %task.agent_profile,
            to = %profile,
            harness_changes,
            "Task agent profile switched"
        );
        Ok(SwitchOutcome::Switched {
            from: task.agent_profile,
        })
    }
}

fn switch_event(
    task_id: &str,
    from: AgentProfile,
    to: AgentProfile,
    note: &SwitchNote<'_>,
    chosen: bool,
) -> InsertTaskEvent {
    InsertTaskEvent {
        event_id: uuid::Uuid::new_v4().to_string(),
        task_id: task_id.to_string(),
        event_type: EventType::Task.as_str().to_string(),
        sub_type: TaskSubType::AgentSwitched.as_str().to_string(),
        is_error: false,
        agent_profile: Some(to.as_str().to_string()),
        turn_number: note.turn_number,
        payload: serde_json::json!({
            "summary": note.summary, "from": from, "to": to, "chosen": chosen
        }),
        timestamp: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
    }
}
