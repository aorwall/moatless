//! The messages a sandbox takes off its queue, and the prompt it runs them
//! with.

use tonic::{Request, Response, Status};
use tracing::{debug, info, warn};

use sandbox_proto::{
    AcknowledgeMessagesRequest, AcknowledgeMessagesResponse, EnqueueSandboxMessageRequest,
    EnqueueSandboxMessageResponse, GetPendingMessagesRequest, GetPendingMessagesResponse,
    GetSystemPromptRequest, GetSystemPromptResponse,
};

use moatless_types::access_control::ScopeContext;

use moatless_tasks::agent::prompt;
use moatless_tasks::task_turn::models::TurnStatus;
use moatless_types::agent::AgentProfile;

use super::state::BackendSandboxServiceImpl;

/// Return pending messages for a task, which the sandbox polls for.
#[tracing::instrument(
    skip_all,
    fields(
        request_id = %uuid::Uuid::new_v4(),
        task_id = tracing::field::Empty,
        user_id = tracing::field::Empty
    )
)]
pub(super) async fn get_pending_messages(
    service: &BackendSandboxServiceImpl,
    request: Request<GetPendingMessagesRequest>,
) -> Result<Response<GetPendingMessagesResponse>, Status> {
    let task_id = request.get_ref().task_id.clone();
    let grant = service.authorizer.authorize(&request, &task_id).await?;
    tracing::Span::current().record("user_id", grant.ctx.user_id.as_str());

    let req = request.into_inner();
    tracing::Span::current().record("task_id", req.task_id.as_str());

    let Some(messages) = service.handlers.as_ref().map(|h| &h.messages) else {
        return Ok(Response::new(GetPendingMessagesResponse {
            deliveries: vec![],
        }));
    };

    let task = if let Some(tasks) = service.handlers.as_ref().map(|h| &h.tasks) {
        // reason: pending-message gate reads task status across all owners (no user ctx)
        #[allow(clippy::disallowed_methods)]
        let sys = ScopeContext::system();
        tasks.find_by_id(&req.task_id, &sys).await.ok().flatten()
    } else {
        None
    };

    // Gate: a Closed task is terminal — never start a turn on it. The
    // sandbox dispatcher no-ops on an empty list, so returning empty here
    // means no `agent.started`, no turn — even if the pod is still finishing
    // its termination grace period. Fail open on unknown (None / DB-error,
    // both collapsed by `.ok().flatten()`): only a confirmed-Closed task is
    // gated, mirroring `send.rs`.
    if matches!(
        task.as_ref().map(|t| t.status),
        Some(moatless_types::task::TaskStatus::Closed)
    ) {
        debug!(task_id = %req.task_id, "get_pending_messages: task is Closed, returning no messages");
        return Ok(Response::new(GetPendingMessagesResponse {
            deliveries: vec![],
        }));
    }

    let task_agent_profile = task.as_ref().map_or_else(
        || AgentProfile::CLAUDE_CODE.to_string(),
        |t| t.agent_profile.0.clone(),
    );
    let task_model = task.as_ref().and_then(|t| t.model.clone());
    let task_skills = task.as_ref().map(|t| t.skills.clone()).unwrap_or_default();

    let moatless_tasks::inbox::view::PendingView {
        messages: pending,
        contexts: contexts_by_message,
    } = messages.pending_user(&req.task_id).await.map_err(|e| {
        warn!(error = %e, task_id = %req.task_id, "Failed to fetch pending messages");
        Status::internal("Failed to fetch pending messages")
    })?;

    // The session `--resume <id>` names when the Trajectory has no head in
    // this harness.
    let latest_agent_session_id = messages
        .latest_agent_session_id(&req.task_id)
        .await
        .unwrap_or(None);

    // Fork intent, while the Task is a fork (or rewound) and has recorded
    // nothing since its cut. Nothing is stamped when intent is *sent*: a first
    // turn that died before recording would consume the stamp and leave the
    // fork permanently unforked.
    let fork_cut = match service.handlers.as_ref() {
        Some(handlers) => match handlers.task_forks.pending(&req.task_id).await {
            Ok(row) => row.map(|row| row.source_event_id),
            Err(e) => {
                // A fork that loses its intent starts on a fresh session and
                // says so in its first answer; one that resumes the parent's
                // session id would write into the parent's transcript.
                warn!(error = %e, task_id = %req.task_id, "Failed to read the fork cut; first turn starts fresh");
                None
            }
        },
        None => None,
    };
    let fork = match (fork_cut, service.handlers.as_ref()) {
        (Some(cut), Some(handlers)) => handlers
            .trajectory
            .cut_anchors(cut)
            .await
            .map_err(|e| trajectory_unavailable(&e, &cut.to_string()))?
            .map(|anchors| sandbox_proto::ForkSession {
                source_message_uuid: anchors.claude_message_id(),
                source_session_id: anchors.harness_session_id,
                source_agent_turn_id: anchors.harness_turn_id,
            }),
        _ => None,
    };

    // Gated on a messages-named session: a Task rewound to turn 0 keeps its
    // Trajectory but continues nothing.
    let switch = match (&fork, &latest_agent_session_id, service.handlers.as_ref()) {
        (Some(_), _, Some(handlers)) => {
            fork_switch(&handlers.trajectory, fork_cut, &task_agent_profile).await?
        }
        (None, Some(_), Some(handlers)) => {
            harness_switch(&handlers.trajectory, &req.task_id, &task_agent_profile).await?
        }
        _ => HarnessSwitch::None,
    };
    let task_resume_session_id = match &switch {
        HarnessSwitch::Resume(head) => Some(head.clone()),
        _ => latest_agent_session_id,
    };

    // Check the last closed turn — a turn that ended `stopped` or `error`
    // cut its agent session off mid-flight, so the next run starts fresh
    // rather than `--resume` into a transcript ending on an unanswered tool
    // call. Turn state is authoritative here: routine stops leave no result
    // row behind (see `message::models::stop_is_reported`).
    let cut_off = match service.ingress.as_ref().map(|i| &i.task_turns) {
        Some(turns) => match turns.latest_closed_turn_status(&req.task_id).await {
            Ok(status) => status.is_some_and(TurnStatus::cut_off_session),
            Err(e) => {
                warn!(error = %e, task_id = %req.task_id, "Failed to read last closed turn; assuming resumable");
                false
            }
        },
        None => false,
    };

    // `will_not_resume` is the explicit refusal. An error row is the implicit
    // one: resuming the same session id would loop the same failure.
    let task_resume_session_id = if task_resume_session_id.is_none() {
        None
    } else if cut_off {
        debug!(
            task_id = %req.task_id,
            "Last closed turn was stopped or errored, clearing resume_session_id"
        );
        None
    } else {
        match messages.get_last_result(&req.task_id).await {
            Ok(Some(last_result)) if last_result.will_not_resume == Some(true) => {
                debug!(
                    task_id = %req.task_id,
                    "Last result has will_not_resume=true, clearing resume_session_id"
                );
                None
            }
            Ok(Some(last_result))
                if last_result.message_type
                    == moatless_tasks::message::models::MessageType::Error =>
            {
                debug!(
                    task_id = %req.task_id,
                    "Last result is an error, clearing resume_session_id to start fresh"
                );
                None
            }
            _ => task_resume_session_id,
        }
    };

    // Pre-resolve the logins `<from>` names, keyed by the writer's own
    // Adapter id: the only field that names the writer rather than the
    // trigger owner.
    let mut login_cache: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    if let Some(handlers) = service.handlers.as_ref() {
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for row in &pending {
            if row.external_event_id.is_some() {
                let (Some(adapter), Some(sender_id)) = (
                    row.external_adapter.as_deref(),
                    row.external_sender_id.as_deref(),
                ) else {
                    continue;
                };
                let key = moatless_tasks::message::enricher::sender_key(adapter, sender_id);
                if !seen.insert(key.clone()) {
                    continue;
                }
                match handlers.provider_link.user_at(adapter, sender_id).await {
                    Ok(Some(user)) => {
                        login_cache.insert(key, user.login);
                    }
                    // Not an error: an unmapped writer is named by their
                    // Adapter handle instead, which stays true.
                    Ok(None) => debug!(
                        adapter,
                        sender_id, "sender is not mapped, <from> names their handle"
                    ),
                    Err(e) => warn!(
                        adapter, sender_id, error = %e,
                        "sender lookup failed, <from> names their handle"
                    ),
                }
            }
        }
    }

    // The agent's view of a queued item is rebuilt here, not read back: the
    // item stores what the person wrote, and everything around it — preview
    // URL, selected element, sending Task, attached files — is its own field.
    let builder = moatless_tasks::message::enricher::AgentPromptBuilder::new(
        service.handlers.as_ref().map(|h| h.files.as_ref()),
        Some(service.backend_config.as_ref()),
        task_agent_profile,
        task_model,
        task_resume_session_id,
        task_skills,
        &login_cache,
    );
    let deliveries = builder
        .build_pending_deliveries(&pending, &contexts_by_message)
        .await
        .into_iter()
        .map(|delivery| sandbox_proto::PendingAgentDelivery {
            message_uuids: delivery.message_uuids,
            prompt_text: delivery.prompt_text,
            file_ids: delivery.file_ids,
            model: delivery.model,
            agent_profile: delivery.agent_profile,
            agent_mode: delivery.agent_mode,
            effort: delivery.effort,
            skills: delivery.skills,
            // A fork's first turn resumes nothing by id and asks for a
            // fresh start, which is what a pod that does not know `fork`
            // falls back to. The alternative — leaving `resume_session_id`
            // set — would have such a pod resume the *parent's* session and
            // write two Tasks into one transcript.
            // A switch starts no CLI session by id either: its session is
            // written out from the Trajectory, rendered into this harness.
            resume_session_id: if fork.is_some() || matches!(switch, HarnessSwitch::Switch) {
                None
            } else {
                delivery.resume_session_id.clone()
            },
            resume: delivery.resume,
            start_fresh: delivery.start_fresh
                || fork.is_some()
                || matches!(switch, HarnessSwitch::Switch),
            fork: fork.clone(),
            switch_harness: matches!(switch, HarnessSwitch::Switch),
            prompt_response: delivery.prompt_response.map(|response| {
                sandbox_proto::PromptResponse {
                    tool_use_id: response.tool_use_id,
                    tool_name: response.tool_name,
                    answers: response.answers.into_iter().collect(),
                    approved: response.approved,
                    feedback: response.feedback,
                }
            }),
        })
        .collect();

    Ok(Response::new(GetPendingMessagesResponse { deliveries }))
}

/// How a Task's next turn continues against its Trajectory's head.
enum HarnessSwitch {
    /// No head, or the head's own session resumes as the messages name it.
    None,
    /// The head is in the profile's harness: its session resumes by this id,
    /// which the messages name wrongly when a switch session was never used.
    Resume(String),
    /// The head, or a fork's cut, is in the other harness: the sandbox opens
    /// a switch (or fork) session rendered into this one, or fails the turn.
    Switch,
}

/// How the Task's next turn continues. A failed read fails the whole
/// `GetPendingMessages`, which the pod retries: resuming by the Messages instead
/// could hand the new harness the old harness's session id.
async fn harness_switch(
    trajectories: &moatless_tasks::trajectory::operations::Trajectories,
    task_id: &str,
    agent_profile: &str,
) -> Result<HarnessSwitch, Status> {
    let Some(harness) = moatless_tasks::agent::catalog::harness_of(agent_profile) else {
        return Ok(HarnessSwitch::None);
    };
    let Some(head) = trajectories
        .head(task_id)
        .await
        .map_err(|e| trajectory_unavailable(&e, task_id))?
    else {
        return Ok(HarnessSwitch::None);
    };
    Ok(if head.harness == harness.as_str() {
        HarnessSwitch::Resume(head.harness_session_id)
    } else {
        HarnessSwitch::Switch
    })
}

/// `Switch` when a fork's (or rewind's) cut is in the other harness's session,
/// so the pod fails the turn rather than start without the fork's history.
async fn fork_switch(
    trajectories: &moatless_tasks::trajectory::operations::Trajectories,
    cut: Option<uuid::Uuid>,
    agent_profile: &str,
) -> Result<HarnessSwitch, Status> {
    let (Some(cut), Some(harness)) = (
        cut,
        moatless_tasks::agent::catalog::harness_of(agent_profile),
    ) else {
        return Ok(HarnessSwitch::None);
    };
    let cut_harness = trajectories
        .event_harness(cut)
        .await
        .map_err(|e| trajectory_unavailable(&e, &cut.to_string()))?;
    Ok(match cut_harness {
        Some(other) if other != harness.as_str() => HarnessSwitch::Switch,
        _ => HarnessSwitch::None,
    })
}

fn trajectory_unavailable(e: &sqlx::Error, subject: &str) -> Status {
    warn!(error = %e, subject, "Failed to read the trajectory; the pod retries");
    Status::unavailable("Failed to read the trajectory")
}

/// Acknowledge that pending messages have been delivered to an agent session.
pub(super) async fn acknowledge_messages(
    service: &BackendSandboxServiceImpl,
    request: Request<AcknowledgeMessagesRequest>,
) -> Result<Response<AcknowledgeMessagesResponse>, Status> {
    let task_id = request.get_ref().task_id.clone();
    service.authorizer.authorize(&request, &task_id).await?;

    let req = request.into_inner();

    let Some(messages) = service.handlers.as_ref().map(|h| &h.messages) else {
        return Ok(Response::new(AcknowledgeMessagesResponse {}));
    };

    // The acknowledging run decides the turn: whichever of the ack and the
    // run's `taskTurn.active` reaches the backend first allocates it, the
    // other joins. Messages injected into a run in flight ack with that
    // run's id and so share its turn.
    let Some(turn_number) = service
        .resolve_turn_for_event(&req.task_id, &req.run_id)
        .await
    else {
        warn!(
            task_id = %req.task_id,
            run_id = %req.run_id,
            "Cannot acknowledge messages without a resolvable turn"
        );
        return Err(Status::internal("Failed to resolve turn for run"));
    };

    match messages
        .acknowledge_messages(
            &req.task_id,
            &req.message_uuids,
            &moatless_tasks::inbox::RunStamp {
                run_id: (!req.run_id.is_empty()).then(|| req.run_id.clone()),
                session_id: req.session_id.clone(),
                turn_number,
            },
        )
        .await
    {
        Ok(count) => {
            info!(
                task_id = %req.task_id,
                session_id = %req.session_id,
                run_id = %req.run_id,
                turn_number,
                requested = req.message_uuids.len(),
                acknowledged = count,
                "Messages acknowledged"
            );
            for uuid in &req.message_uuids {
                moatless_tasks::message::hooks::emit_message_upsert_from_daos(
                    service.events.as_ref(),
                    messages,
                    service.handlers.as_ref().map(|h| &h.files),
                    &req.task_id,
                    uuid,
                )
                .await;
            }
        }
        Err(e) => {
            warn!(
                task_id = %req.task_id,
                error = %e,
                "Failed to acknowledge messages"
            );
            return Err(Status::internal("Failed to acknowledge messages"));
        }
    }

    let answered_prompt = (!req.answered_prompt_tool_use_id.is_empty())
        .then(|| req.message_uuids.first())
        .flatten();
    if let Some(carrier) = answered_prompt {
        moatless_tasks::message::send::settle_answered_prompt(
            messages.as_ref(),
            &req.task_id,
            &req.answered_prompt_tool_use_id,
            carrier,
        )
        .await;
    }

    Ok(Response::new(AcknowledgeMessagesResponse {}))
}

/// Queue a user message the sandbox wrote for its own Task.
///
/// The row makes the turn visible to `find_interrupted_turn`; waking the agent
/// handle alone leaves none. Its inbox `origin` is `sandbox`, which no
/// conversation shows.
pub(super) async fn enqueue_sandbox_message(
    service: &BackendSandboxServiceImpl,
    request: Request<EnqueueSandboxMessageRequest>,
) -> Result<Response<EnqueueSandboxMessageResponse>, Status> {
    let task_id = request.get_ref().task_id.clone();
    service.authorizer.authorize(&request, &task_id).await?;
    let req = request.into_inner();

    if req.text.trim().is_empty() {
        return Err(Status::invalid_argument("text is required"));
    }

    let (messages, tasks) = service
        .handlers
        .as_ref()
        .map(|h| (h.messages.clone(), h.tasks.clone()))
        .ok_or_else(|| Status::failed_precondition("Database not configured"))?;

    let queued = moatless_tasks::message::send::enqueue_sandbox_message(
        messages.as_ref(),
        tasks.as_ref(),
        &task_id,
        &req.text,
        (!req.idempotency_key.is_empty()).then_some(req.idempotency_key.as_str()),
    )
    .await
    .map_err(|error| {
        warn!(task_id, %error, "failed to enqueue sandbox message");
        Status::internal("Failed to enqueue sandbox message")
    })?;

    Ok(Response::new(EnqueueSandboxMessageResponse {
        message_uuid: queued.unwrap_or_default(),
    }))
}

/// Build and return the assembled append-system-prompt for a task.
///
/// Called once per agent run, in `start_session`; the sandbox caches the result
/// as the fallback for a later failure.
pub(super) async fn get_system_prompt(
    service: &BackendSandboxServiceImpl,
    request: Request<GetSystemPromptRequest>,
) -> Result<Response<GetSystemPromptResponse>, Status> {
    // The assembled prompt embeds task and repo details, so it is owner-only.
    let task_id = request.get_ref().task_id.clone();
    let grant = service.authorizer.authorize(&request, &task_id).await?;
    let req = request.into_inner();

    let task = match service.handlers.as_ref().map(|h| &h.tasks) {
        Some(tasks) => tasks
            .find_by_id(&req.task_id, &grant.ctx)
            .await
            .ok()
            .flatten(),
        None => None,
    };

    let system_prompt = prompt::build_for_task(
        &req.task_id,
        task.as_ref(),
        service.handlers.as_ref().map(|h| &h.workspace_resolver),
        service.handlers.as_ref().map(|h| h.task_relations.as_ref()),
        service.handlers.as_ref().map(|h| h.user_logins.as_ref()),
        Some(&service.backend_config),
    )
    .await;

    Ok(Response::new(GetSystemPromptResponse { system_prompt }))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    /// Trajectories over a database that never answers.
    fn unreachable() -> std::sync::Arc<moatless_tasks::trajectory::operations::Trajectories> {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .acquire_timeout(Duration::from_millis(100))
            .connect_lazy("postgres://postgres@127.0.0.1:1/none")
            .expect("a lazy pool");
        moatless_tasks::compose::task_graph(pool).trajectory()
    }

    #[tokio::test]
    async fn a_failed_head_read_fails_the_delivery() {
        let refused = harness_switch(&unreachable(), "t", AgentProfile::CODEX)
            .await
            .err()
            .expect("the pod retries rather than resume by the Messages");
        assert_eq!(refused.code(), tonic::Code::Unavailable);
    }

    #[tokio::test]
    async fn a_failed_fork_cut_read_fails_the_delivery() {
        let refused = fork_switch(
            &unreachable(),
            Some(uuid::Uuid::nil()),
            AgentProfile::CLAUDE_CODE,
        )
        .await
        .err()
        .expect("the pod retries rather than fork without the history");
        assert_eq!(refused.code(), tonic::Code::Unavailable);
    }
}
