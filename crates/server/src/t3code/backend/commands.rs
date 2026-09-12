//! [`ThreadCommandSink`]: every command a T3 thread sends, as the service call
//! the Moatless UI makes for the same thing.

use super::attachments::ResolvedAttachments;
use super::pull_requests::{bind_thread_to_open_pr, github_owner_repo};
use crate::state::AppState;
use async_trait::async_trait;
use moatless_t3code::ports::{
    CommandRefusal, ForkThread, NewThread, QuestionAnswer, ThreadCommandSink, ThreadMeta,
    ThreadPullRequestRef, ThreadTurn,
};
use moatless_tasks::message::error::MessageError;
use moatless_tasks::message::service as message_service;
use moatless_tasks::task::error::TaskError;
use moatless_tasks::task::models::Task;
use moatless_tasks::task::models::TaskPatch;
use moatless_tasks::task::models::TaskStatus;
use moatless_tasks::task::operations::TaskCommands;
use moatless_tasks::task::service as task_service;
use moatless_types::access_control::ScopeContext;
use moatless_types::access_control::ScopedResource;
use moatless_types::task::Visibility;
use std::sync::Arc;

pub struct AppStateCommands(pub AppState);

impl AppStateCommands {
    pub fn new(state: AppState) -> Arc<Self> {
        Arc::new(Self(state))
    }

    /// A viewer as the message and task services expect one.
    ///
    /// `is_agent_context` is false because every command on this socket comes
    /// from a person looking at a screen — which is what makes the auto-follow
    /// on send correct here.
    pub(super) fn sender(viewer: &ScopeContext) -> message_service::Sender<'_> {
        message_service::Sender {
            user_id: &viewer.user_id,
            scope: viewer,
            is_agent_context: false,
        }
    }

    /// The Task under the viewer's own access filter, refusing one they may not
    /// read as `NotWritable` rather than reporting it missing.
    async fn readable_task(
        &self,
        viewer: &ScopeContext,
        task_id: &str,
    ) -> Result<Task, CommandRefusal> {
        self.0
            .tasks
            .find_by_id(task_id, viewer)
            .await
            .map_err(|error| {
                tracing::warn!(%error, task_id, "T3 thread command could not resolve its thread");
                CommandRefusal::Failed {
                    message: "Could not reach that thread.".to_string(),
                }
            })?
            .ok_or_else(|| CommandRefusal::NotWritable {
                task_id: task_id.to_string(),
            })
    }

    async fn ensure_followed_open(
        &self,
        viewer: &ScopeContext,
        task_id: &str,
    ) -> Result<(), CommandRefusal> {
        let task = self.readable_task(viewer, task_id).await?;
        if task.status != TaskStatus::Open {
            return Err(CommandRefusal::NotWritable {
                task_id: task_id.to_string(),
            });
        }
        let followed = self
            .0
            .tasks
            .get_followed(&viewer.user_id, &[task_id.to_string()])
            .await
            .map_err(|error| {
                tracing::warn!(%error, task_id, "T3 settlement command could not resolve follow");
                CommandRefusal::Failed {
                    message: "Could not reach that thread.".to_string(),
                }
            })?;
        if !followed.contains(task_id) {
            return Err(CommandRefusal::NotWritable {
                task_id: task_id.to_string(),
            });
        }
        Ok(())
    }

    /// Switch one pull request Binding's routing, reporting success whether or
    /// not the row moved — both commands are idempotent, and the client sends
    /// either one against the state it already believes.
    async fn set_pull_request_routing(
        &self,
        reference: &ThreadPullRequestRef,
        active: bool,
    ) -> Result<(), CommandRefusal> {
        let subject_key = format!("{}#{}", reference.repository, reference.number);
        self.0
            .task_bindings
            .set_routing(
                &reference.task_id,
                "github",
                "github_pr",
                &subject_key,
                active,
            )
            .await
            .map(|_| ())
            .map_err(|error| {
                tracing::warn!(
                    %error,
                    task_id = %reference.task_id,
                    subject_key,
                    active,
                    "T3 pull request routing change failed"
                );
                CommandRefusal::Failed {
                    message: "That pull request could not be updated.".to_string(),
                }
            })
    }

    fn state_refusal(task_id: &str, error: sqlx::Error) -> CommandRefusal {
        tracing::warn!(%error, task_id, "T3 per-viewer thread state update failed");
        CommandRefusal::Failed {
            message: "Could not update that thread.".to_string(),
        }
    }

    /// Whether this turn's Message is already persisted — the frame is a
    /// replay. The send path makes the same read and is what actually resolves
    /// the replay, so a lookup that fails answers `false` and leaves that
    /// decision where it belongs rather than refusing a turn over it.
    async fn already_sent(&self, turn: &ThreadTurn) -> bool {
        match self.0.messages.find_by_uuid(&turn.message_uuid).await {
            Ok(existing) => existing.is_some_and(|message| message.task_id == turn.task_id),
            Err(error) => {
                tracing::warn!(%error, task_id = %turn.task_id, "T3 turn could not be checked for replay");
                false
            }
        }
    }
}

#[async_trait]
impl ThreadCommandSink for AppStateCommands {
    async fn send_turn(
        &self,
        viewer: &ScopeContext,
        turn: ThreadTurn,
    ) -> Result<(), CommandRefusal> {
        // Stored before the Message, so a turn that reaches the agent always
        // names files that exist.
        //
        // A replay has to be caught here rather than left to the send path: the
        // send resolves it by reading the Message already under that uuid and
        // writing nothing, but by then these bytes are in S3 under a file id no
        // Message will ever name, and a reconnecting client replays whole
        // turns. The extra read only happens on turns that carry files.
        let file_ids = if turn.attachments.is_empty() || self.already_sent(&turn).await {
            Vec::new()
        } else {
            self.resolve_attachments(viewer, &turn.attachments)
                .await?
                .file_ids
        };

        message_service::send_user_message(
            self.0.tasks.as_ref(),
            self.0.message_send.as_ref(),
            self.0.sandbox_waker.as_ref(),
            &Self::sender(viewer),
            &turn.task_id,
            message_service::UserMessage {
                text: turn.text,
                model: turn.model,
                agent_mode: turn.agent_mode,
                effort: turn.effort,
                resume: true,
                message_uuid: Some(turn.message_uuid),
                file_ids,
                skills: turn.skills,
                ..Default::default()
            },
        )
        .await
        .map(|_| ())
        .map_err(|e| send_refusal(&turn.task_id, e))
    }

    async fn create_thread(
        &self,
        viewer: &ScopeContext,
        thread: NewThread,
    ) -> Result<(), CommandRefusal> {
        // Before anything is stored: the id arrives from the client and may
        // still be held by a creation that failed, and the attachments below go
        // to S3 under a file id no Message would ever name if the create is
        // refused here.
        free_thread_id(self.0.tasks.as_ref(), viewer, &thread.task_id).await?;

        // The transport is not selectable from T3, so a thread created there
        // runs the first catalog agent of the kind the person picked.
        let agent_type = default_agent_for(&thread.instance);

        // Kept past the request, which moves both: a thread created on a PR's
        // head branch is bound to that PR once the Task exists.
        let workspace_id = thread.workspace_id.clone();
        let branch = thread.branch.clone();

        let attachments = match thread.first_message.as_ref() {
            Some(first) => self.resolve_attachments(viewer, &first.attachments).await?,
            None => ResolvedAttachments {
                file_ids: Vec::new(),
                stored_here: Vec::new(),
            },
        };

        let request = moatless_api_schemas::task::CreateTaskRequest {
            repository_id: None,
            workspace_id: Some(thread.workspace_id),
            sandbox_id: None,
            checkout_mode: None,
            agent_type: Some(agent_type),
            // Created unnamed: the agent names it on its first turn, and the
            // client's create title is a placeholder.
            name: None,
            description: None,
            branch: thread.branch,
            message: thread.first_message.as_ref().map(|m| m.text.clone()),
            model: thread.model,
            agent_mode: thread
                .first_message
                .as_ref()
                .and_then(|m| m.agent_mode.clone()),
            effort: thread.first_message.as_ref().and_then(|m| m.effort.clone()),
            contexts: vec![],
            file_ids: attachments.file_ids.clone(),
            parent_task_id: None,
            // The T3 surface has no run-as control; a task it opens runs as its caller.
            run_as_user_id: None,
            headless: Some(false),
            skills: thread
                .first_message
                .as_ref()
                .map(|message| message.skills.clone())
                .unwrap_or_default(),
        };

        let mut intent = task_service::CreateTaskIntent::new(request);
        intent.task_id = Some(thread.task_id.clone());
        // A thread created with nothing in it gets no sandbox until something
        // is sent to it, unless the create asked for one — see
        // [`NewThread::provision`].
        intent.provision = thread.first_message.is_some() || thread.provision;
        // T3's optimistic copy of the first message is keyed on the id it
        // minted, so that id has to be the one persisted.
        intent.first_message_uuid = thread.first_message.map(|m| m.message_uuid);

        // A create that fails writes no Message, so nothing will ever name the
        // inline bytes stored above — including the preflight failure this
        // thread id is being retried after, which would otherwise leave one
        // unreachable object per retry. Only what this call stored is taken
        // back; an uploaded attachment is the client's and outlives the retry.
        let new_task = match task_service::normalize(
            task_service::CreateTaskPorts {
                workspaces: self.0.workspace_resolver.as_ref(),
                repositories: self.0.repository_reader.as_ref(),
                tasks: self.0.tasks.as_ref(),
                users: self.0.user_directory.as_ref(),
                credential_sync: &self.0.credential_sync,
                config: &self.0.config,
            },
            viewer,
            &viewer.user_id,
            intent,
        )
        .await
        {
            Ok(new_task) => new_task,
            Err(e) => {
                self.discard_attachments(viewer, &attachments.stored_here)
                    .await;
                return Err(create_refusal(&thread.task_id, e));
            }
        };
        if let Err(e) = task_service::create_task(
            &self.0.task_creation,
            self.0.message_send.clone(),
            new_task,
            |_| async { serde_json::Value::Null },
        )
        .await
        {
            self.discard_attachments(viewer, &attachments.stored_here)
                .await;
            return Err(create_refusal(&thread.task_id, e));
        }

        // The PR-prepared thread flow drops the pull-request reference before
        // `thread.create` (ChatView.tsx), so the head branch is the only signal
        // that this thread belongs to a pull request. Bind it here from that
        // branch so a review comment on the PR is delivered into this Task.
        if let Some(branch) = branch.as_deref().filter(|branch| !branch.is_empty()) {
            bind_thread_to_open_pr(&self.0, viewer, &workspace_id, branch, &thread.task_id).await;
        }
        Ok(())
    }

    async fn fork(&self, viewer: &ScopeContext, fork: ForkThread) -> Result<(), CommandRefusal> {
        // Same reason `create_thread` checks this first: the id arrives from
        // the client, and `copy_conversation` below writes rows keyed on it
        // before any Task row exists to claim it.
        free_thread_id(self.0.tasks.as_ref(), viewer, &fork.new_task_id).await?;

        let (source, plan) = self
            .0
            .task_fork
            .plan(viewer, &fork.source_task_id, fork.at_turn)
            .await
            .map_err(|e| create_refusal(&fork.source_task_id, e))?;

        let copied = self
            .0
            .task_fork
            .copy_conversation(&plan, &fork.new_task_id)
            .await
            .map_err(|e| create_refusal(&fork.new_task_id, e))?;

        let request = moatless_api_schemas::task::CreateTaskRequest {
            repository_id: Some(source.repository_id.clone()),
            workspace_id: source.workspace_id.clone(),
            sandbox_id: fork
                .same_sandbox
                .then(|| source.sandbox_id.clone())
                .flatten(),
            checkout_mode: fork
                .same_sandbox
                .then_some(moatless_types::task::CheckoutMode::Shared),
            agent_type: Some(source.agent_type.clone()),
            name: None,
            description: None,
            branch: source.branch.clone(),
            message: fork.message.clone(),
            model: fork.model.clone().or_else(|| source.model.clone()),
            agent_mode: fork.agent_mode.clone(),
            effort: fork.effort.clone(),
            contexts: Vec::new(),
            file_ids: Vec::new(),
            parent_task_id: None,
            // The T3 surface has no run-as control; a task it opens runs as its caller.
            run_as_user_id: None,
            headless: None,
            // T3 has no skill picker of its own.
            skills: Vec::new(),
        };

        let create_result = async {
            let mut new_task = task_service::normalize(
                task_service::CreateTaskPorts {
                    workspaces: self.0.workspace_resolver.as_ref(),
                    repositories: self.0.repository_reader.as_ref(),
                    tasks: self.0.tasks.as_ref(),
                    users: self.0.user_directory.as_ref(),
                    credential_sync: &self.0.credential_sync,
                    config: &self.0.config,
                },
                viewer,
                &viewer.user_id,
                task_service::CreateTaskIntent {
                    request,
                    task_id: Some(fork.new_task_id.clone()),
                    provision: true,
                    first_message_uuid: None,
                },
            )
            .await?;
            new_task.tag_ids = self.0.task_fork.source_tag_ids(&fork.source_task_id).await;
            new_task.forked_from = Some(moatless_tasks::task::creation::ForkedFrom {
                source_task_id: fork.source_task_id.clone(),
                at_turn: plan.at_turn,
                session: plan.session.clone(),
            });
            task_service::create_task(
                &self.0.task_creation,
                self.0.message_send.clone(),
                new_task,
                |_| async { serde_json::Value::Null },
            )
            .await
        }
        .await;

        if let Err(e) = create_result {
            self.0
                .task_fork
                .discard_conversation(&fork.new_task_id)
                .await;
            return Err(create_refusal(&fork.new_task_id, e));
        }

        tracing::info!(
            task_id = %fork.new_task_id,
            source_task_id = %fork.source_task_id,
            at_turn = ?plan.at_turn,
            messages = copied.messages,
            activities = copied.activities,
            contexts = copied.contexts,
            same_sandbox = fork.same_sandbox,
            "Task forked from T3"
        );
        Ok(())
    }

    async fn revert(
        &self,
        viewer: &ScopeContext,
        task_id: &str,
        keep_through: i32,
    ) -> Result<(), CommandRefusal> {
        self.0
            .task_rewind
            .rewind(viewer, task_id, keep_through)
            .await
            .map_err(|e| create_refusal(task_id, e))
    }

    async fn update_meta(
        &self,
        viewer: &ScopeContext,
        meta: ThreadMeta,
    ) -> Result<(), CommandRefusal> {
        if meta.title.is_none() && meta.model.is_none() {
            return Ok(());
        }
        let patch = TaskPatch {
            name: meta.title,
            model: meta.model,
            ..Default::default()
        };
        let updated = self
            .0
            .tasks
            .update_task(&meta.task_id, &patch, viewer)
            .await
            .map_err(|error| {
                tracing::warn!(%error, task_id = %meta.task_id, "T3 meta update failed");
                CommandRefusal::Failed {
                    message: "Could not update that thread.".to_string(),
                }
            })?;
        // `update_task` carries the access rule in its `WHERE`, so a viewer who
        // may not control the Task gets a clean `false` rather than an error.
        // Reporting that as success would leave T3 showing a title nothing
        // stored.
        if !updated {
            return Err(CommandRefusal::NotWritable {
                task_id: meta.task_id,
            });
        }
        Ok(())
    }

    async fn set_idle_timeout(
        &self,
        viewer: &ScopeContext,
        task_id: &str,
        minutes: i32,
    ) -> Result<(), CommandRefusal> {
        let patch = TaskPatch {
            ttl_minutes: Some(minutes),
            ..Default::default()
        };
        let updated = self
            .0
            .tasks
            .update_task(task_id, &patch, viewer)
            .await
            .map_err(|error| {
                tracing::warn!(%error, task_id, "T3 idle timeout update failed");
                CommandRefusal::Failed {
                    message: "Could not change that sandbox's idle timeout.".to_string(),
                }
            })?;
        // Same clean `false` as `update_meta`, and the same reason for turning
        // it into a refusal.
        if !updated {
            return Err(CommandRefusal::NotWritable {
                task_id: task_id.to_string(),
            });
        }
        Ok(())
    }

    async fn set_visibility(
        &self,
        viewer: &ScopeContext,
        task_id: &str,
        visibility: Visibility,
    ) -> Result<(), CommandRefusal> {
        let patch = TaskPatch {
            visibility: Some(visibility),
            ..Default::default()
        };
        let updated = self
            .0
            .tasks
            .update_task(task_id, &patch, viewer)
            .await
            .map_err(|error| {
                tracing::warn!(%error, task_id, "T3 visibility update failed");
                CommandRefusal::Failed {
                    message: "Could not change that thread's visibility.".to_string(),
                }
            })?;
        // Same clean `false` as `update_meta`, and the same reason for turning
        // it into a refusal.
        if !updated {
            return Err(CommandRefusal::NotWritable {
                task_id: task_id.to_string(),
            });
        }
        // `info!`, matching the HTTP twin in `tasks::task::handlers`: the
        // transition opens the Task's Sandbox to every signed-in viewer, and
        // which surface it was made from must not decide whether it is
        // recorded. `orchestration_dispatch_command` logs refusals only.
        tracing::info!(
            task_id,
            user_id = %viewer.user_id,
            %visibility,
            "Task visibility updated from T3"
        );
        Ok(())
    }

    async fn interrupt(&self, viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal> {
        task_service::stop_agent(
            self.0.tasks.as_ref(),
            self.0.sandbox.as_ref(),
            self.0.events.as_deref(),
            viewer,
            task_id,
        )
        .await
        .map_err(|e| create_refusal(task_id, e))
    }

    async fn stop(&self, viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal> {
        task_service::stop_task(
            self.0.tasks.as_ref(),
            self.0.sandbox.as_ref(),
            self.0.events.as_deref(),
            viewer,
            task_id,
        )
        .await
        .map_err(|e| create_refusal(task_id, e))
    }

    async fn archive(&self, viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal> {
        self.0
            .task_closer
            .close_for(viewer, task_id)
            .await
            .map_err(|e| create_refusal(task_id, e))
    }

    async fn unarchive(&self, viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal> {
        task_service::reopen_task_for(
            self.0.tasks.as_ref(),
            self.0.events.as_deref(),
            viewer,
            task_id,
        )
        .await
        .map_err(|e| create_refusal(task_id, e))
    }

    async fn follow(&self, viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal> {
        // The same gate the HTTP twin in `tasks::task::handlers` uses: a Task
        // the viewer cannot read must not be reachable by following it, and a
        // follow row for a Task that does not exist would list nothing.
        self.readable_task(viewer, task_id).await?;
        self.0
            .tasks
            .follow_task(&viewer.user_id, task_id)
            .await
            .map_err(|e| Self::state_refusal(task_id, e))?;
        // `info!` for the reason `set_visibility` gives, and matching the same
        // pair of lines in the HTTP twin.
        tracing::info!(task_id, user_id = %viewer.user_id, "Task followed from T3");
        Ok(())
    }

    async fn unfollow(&self, viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal> {
        // No readability check, deliberately unlike `follow` and unlike the
        // HTTP twin: this deletes the viewer's own follow row, and a Task that
        // went private or was deleted underneath them is exactly the one they
        // still need to be able to drop. `docs/deferred-changes.md` holds the
        // HTTP side.
        self.0
            .tasks
            .unfollow_task(&viewer.user_id, task_id)
            .await
            .map_err(|e| Self::state_refusal(task_id, e))?;
        // See `follow`.
        tracing::info!(task_id, user_id = %viewer.user_id, "Task unfollowed from T3");
        Ok(())
    }

    async fn settle(&self, viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal> {
        self.ensure_followed_open(viewer, task_id).await?;
        self.0
            .tasks
            .settle_task(&viewer.user_id, task_id, chrono::Utc::now())
            .await
            .map_err(|e| Self::state_refusal(task_id, e))?;
        // Settling says nobody is waiting on this thread any more, so the
        // sandbox it was holding is released rather than left for the idle
        // reaper. Best-effort: the settlement is already durable, and a viewer
        // who follows a thread they may not control has no sandbox to stop —
        // neither case is a reason to report the settle as refused. New
        // activity clears the settlement (`clear_settled_for_all_followers`)
        // and the next turn starts the sandbox again.
        if let Err(error) = task_service::stop_task(
            self.0.tasks.as_ref(),
            self.0.sandbox.as_ref(),
            self.0.events.as_deref(),
            viewer,
            task_id,
        )
        .await
        {
            tracing::warn!(%error, task_id, "Sandbox stop on settle failed");
        }
        Ok(())
    }

    async fn unsettle(&self, viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal> {
        self.ensure_followed_open(viewer, task_id).await?;
        self.0
            .tasks
            .unsettle_task(&viewer.user_id, task_id)
            .await
            .map_err(|e| Self::state_refusal(task_id, e))
    }

    async fn snooze(
        &self,
        viewer: &ScopeContext,
        task_id: &str,
        snoozed_until: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), CommandRefusal> {
        self.ensure_followed_open(viewer, task_id).await?;
        self.0
            .tasks
            .snooze_task(&viewer.user_id, task_id, snoozed_until, chrono::Utc::now())
            .await
            .map_err(|e| Self::state_refusal(task_id, e))
    }

    async fn unsnooze(&self, viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal> {
        self.ensure_followed_open(viewer, task_id).await?;
        self.0
            .tasks
            .unsnooze_task(&viewer.user_id, task_id)
            .await
            .map_err(|e| Self::state_refusal(task_id, e))
    }

    async fn answer_question(
        &self,
        viewer: &ScopeContext,
        answer: QuestionAnswer,
    ) -> Result<(), CommandRefusal> {
        message_service::send_user_message(
            self.0.tasks.as_ref(),
            self.0.message_send.as_ref(),
            self.0.sandbox_waker.as_ref(),
            &Self::sender(viewer),
            &answer.task_id,
            message_service::UserMessage {
                resume: true,
                tool_response: Some(message_service::ToolResponse {
                    tool_use_id: answer.tool_use_id,
                    tool_name: answer.tool_name,
                    payload: answer.payload,
                }),
                ..Default::default()
            },
        )
        .await
        .map(|_| ())
        .map_err(|e| send_refusal(&answer.task_id, e))
    }

    async fn link_pull_request(
        &self,
        viewer: &ScopeContext,
        reference: ThreadPullRequestRef,
    ) -> Result<(), CommandRefusal> {
        let (owner, repo) = github_owner_repo(&reference)?;
        let Some(connector) = self
            .0
            .adapter_installations
            .as_ref()
            .map(|installations| installations.github_connector())
        else {
            return Err(CommandRefusal::Failed {
                message: "GitHub is not configured on this server.".to_string(),
            });
        };
        connector
            .connect_github_binding(
                &reference.repository,
                &reference.task_id,
                serde_json::json!({
                    "owner": owner,
                    "repo": repo,
                    "pr_number": reference.number,
                }),
                Some(&viewer.user_id),
            )
            .await
            .map_err(|error| {
                tracing::warn!(%error, task_id = %reference.task_id, "T3 pull request link failed");
                CommandRefusal::Failed {
                    message: "That pull request could not be linked.".to_string(),
                }
            })?;
        // A Binding released by an earlier unlink is re-bound rather than
        // re-created — `connect_github_binding` upserts on the same key and
        // leaves `active` alone — so linking it again has to switch routing back
        // on for the row to reach `pullRequests`.
        self.set_pull_request_routing(&reference, true).await
    }

    async fn unlink_pull_request(
        &self,
        _viewer: &ScopeContext,
        reference: ThreadPullRequestRef,
    ) -> Result<(), CommandRefusal> {
        github_owner_repo(&reference)?;
        self.set_pull_request_routing(&reference, false).await
    }
}

/// The agent a thread created from T3 runs.
pub(super) fn default_agent_for(instance: &str) -> moatless_types::agent::AgentType {
    let id = moatless_tasks::agent::catalog::catalog()
        .iter()
        .find(|entry| agent_instance_for(entry.kind) == instance)
        .map_or(moatless_types::agent::AgentType::CLAUDE_CODE, |entry| {
            entry.id
        });
    moatless_types::agent::AgentType(id.to_string())
}

fn agent_instance_for(kind: moatless_tasks::agent::catalog::AgentKind) -> &'static str {
    match kind {
        moatless_tasks::agent::catalog::AgentKind::ClaudeCode => "claudeAgent",
        moatless_tasks::agent::catalog::AgentKind::Codex => "codex",
    }
}

fn send_refusal(task_id: &str, error: MessageError) -> CommandRefusal {
    match error {
        MessageError::NotFound(_) => CommandRefusal::NotWritable {
            task_id: task_id.to_string(),
        },
        MessageError::TaskClosed(_) => CommandRefusal::Rejected {
            message: "This task is closed and cannot take new messages.".to_string(),
        },
        MessageError::BadRequest(message) => CommandRefusal::Rejected { message },
        other => {
            tracing::warn!(error = %other, task_id, "T3 send failed");
            CommandRefusal::Failed {
                message: "The message could not be sent.".to_string(),
            }
        }
    }
}

/// Make the id a `thread.create` names available, or say why it is not.
///
/// T3's client mints the thread id and Moatless persists it verbatim, so an id
/// can arrive already taken by the Task a failed creation left behind: a
/// preflight failure persists the row as `Error` and *then* returns the error.
/// The client cannot escape that on its own — it rotates a draft id only on a
/// `thread.deleted` event, which this server never emits, and `thread.delete`
/// is refused — so without this the retry hits the unique index on
/// `tasks.task_id` and the draft is dead for as long as the person keeps it.
///
/// Only that incarnation is cleared, and the delete re-checks the condition in
/// its own `WHERE`: an id held by a Task that is alive or has a conversation in
/// it is refused, so `thread.create` never becomes a way to discard a thread.
/// The ownership check here is what keeps the refusal for somebody else's id
/// indistinguishable from an id this viewer may not read.
pub(super) async fn free_thread_id(
    tasks: &TaskCommands,
    viewer: &ScopeContext,
    task_id: &str,
) -> Result<(), CommandRefusal> {
    let existing = tasks.find_by_id(task_id, viewer).await.map_err(|error| {
        tracing::warn!(%error, task_id, "T3 thread create could not read the id it was given");
        CommandRefusal::Failed {
            message: "The thread could not be created.".to_string(),
        }
    })?;
    // Free, or held by a Task this viewer cannot read. In the second case the
    // unique index refuses the create, which is the same answer without
    // confirming that the id is taken.
    let Some(existing) = existing else {
        return Ok(());
    };

    if !viewer.can_write(&ScopedResource::borrowed(
        existing.scope,
        existing
            .owner_user_id
            .as_deref()
            .or(existing.created_by.as_deref()),
    )) {
        return Err(CommandRefusal::NotWritable {
            task_id: task_id.to_string(),
        });
    }

    let reclaimed = tasks
        .delete_errored_empty_task(task_id, viewer)
        .await
        .map_err(|error| {
            tracing::warn!(%error, task_id, "T3 thread create could not reclaim a failed bootstrap");
            CommandRefusal::Failed {
                message: "The thread could not be created.".to_string(),
            }
        })?;
    if !reclaimed {
        return Err(CommandRefusal::Rejected {
            message: "That thread already exists.".to_string(),
        });
    }

    tracing::info!(
        task_id,
        "T3 thread create reclaimed a failed bootstrap's id"
    );
    Ok(())
}

pub(super) fn create_refusal(task_id: &str, error: TaskError) -> CommandRefusal {
    match error {
        TaskError::NotFound(_) | TaskError::Forbidden(_) => CommandRefusal::NotWritable {
            task_id: task_id.to_string(),
        },
        TaskError::BadRequest(message) => CommandRefusal::Rejected { message },
        // Understood, and refused for something the person can act on — a turn
        // still in flight, a lifecycle operation already holding the Sandbox.
        // The message carries through because it names what to do next, and
        // because the HTTP surface already answers these with the same string
        // as its 409 body, so repeating it here discloses nothing further.
        TaskError::Conflict(message) => CommandRefusal::Rejected { message },
        TaskError::RepoNotFound(id) => CommandRefusal::Rejected {
            message: format!("Repository {id} could not be found."),
        },
        other => {
            tracing::warn!(error = %other, task_id, "T3 task command failed");
            CommandRefusal::Failed {
                message: "The action could not be completed.".to_string(),
            }
        }
    }
}
