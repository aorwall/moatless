//! `AppendTrajectory` and `ReadTrajectory`: the trajectory a sandbox records
//! its harness session file into, and writes it back out from.

use moatless_types::agent::Harness;
use std::pin::Pin;

use futures::Stream;
use tonic::{Request, Response, Status};
use tracing::warn;

use sandbox_proto::{
    AppendTrajectoryRequest, AppendTrajectoryResponse, ReadTrajectoryChunk, ReadTrajectoryRequest,
    SessionHeader, TranscodeItem, read_trajectory_chunk, read_trajectory_request,
};

use moatless_tasks::message::hooks::{emit_message_upsert_from_daos, emit_subagent_progress};
use moatless_tasks::trajectory::models::{
    AppendError, AppendRequest, ChainError, LineProjection, NewEvent, OpenedSession, SubagentParent,
};
use moatless_tasks::trajectory::operations::Trajectories;

use super::state::BackendSandboxServiceImpl;
use crate::event_handler::metrics::{bucket_tool, record_tool_call};

/// The chunk stream [`read_trajectory`] answers with.
pub(super) type TrajectoryChunkStream =
    Pin<Box<dyn Stream<Item = Result<ReadTrajectoryChunk, Status>> + Send>>;

fn store(service: &BackendSandboxServiceImpl) -> Result<&Trajectories, Status> {
    service
        .handlers
        .as_ref()
        .map(|h| h.trajectory.as_ref())
        .ok_or_else(|| Status::unimplemented("trajectory store needs a database"))
}

fn harness(value: &str) -> Result<Harness, Status> {
    Harness::parse(value).ok_or_else(|| Status::invalid_argument("unknown harness"))
}

fn internal(e: impl std::fmt::Display, what: &'static str) -> Status {
    warn!(error = %e, "{what}");
    Status::internal(what)
}

pub(super) async fn append_trajectory(
    service: &BackendSandboxServiceImpl,
    request: Request<AppendTrajectoryRequest>,
) -> Result<Response<AppendTrajectoryResponse>, Status> {
    let task_id = request.get_ref().task_id.clone();
    service.authorizer.authorize(&request, &task_id).await?;
    let dao = store(service)?;
    let req = request.into_inner();
    let harness = harness(&req.harness)?;
    if req.harness_session_id.is_empty() || req.first_seq < 0 {
        return Err(Status::invalid_argument(
            "harness_session_id and a non-negative first_seq are required",
        ));
    }
    let lines = req
        .lines
        .into_iter()
        .map(|line| {
            let content = serde_json::from_str(&line.content_json)
                .map_err(|_| Status::invalid_argument("content_json is not JSON"))?;
            Ok(NewEvent {
                kind: line.kind,
                harness_item_id: line.harness_item_id,
                harness_turn_id: line.harness_turn_id,
                payload: line.payload,
                content,
            })
        })
        .collect::<Result<Vec<_>, Status>>()?;

    // The sandbox does not know the Turn's number, so the latest one is stamped
    // here; a line flushed after its Turn closed can carry the next Turn's.
    let turn_number = match (req.turn_number, service.ingress.as_ref()) {
        (Some(n), _) => Some(n),
        (None, Some(ingress)) if !lines.is_empty() => ingress
            .task_turns
            .latest_turn_number(&task_id)
            .await
            .map_err(|e| internal(e, "Failed to read the running Turn"))?,
        _ => None,
    };
    let outcome = dao
        .append(AppendRequest {
            task_id: &task_id,
            harness,
            harness_session_id: &req.harness_session_id,
            first_seq: req.first_seq,
            lines: &lines,
            turn_number,
            subagent_parent: req.subagent_parent.as_ref().map(|p| SubagentParent {
                harness_session_id: &p.parent_harness_session_id,
                tool_use_id: &p.tool_use_id,
            }),
            own_from_seq: req.own_from_seq,
        })
        .await
        .map_err(|e| match e {
            AppendError::Foreign(_) => Status::permission_denied(e.to_string()),
            AppendError::ParentMissing(_) => Status::failed_precondition(e.to_string()),
            AppendError::OwnFromSeqMissing(_) => Status::invalid_argument(e.to_string()),
            AppendError::Db(e) => internal(e, "Failed to append trajectory"),
        })?;
    notify_projection(service, &task_id, harness, &outcome.projected).await;
    Ok(Response::new(AppendTrajectoryResponse {
        accepted: outcome.accepted,
        last_seq: outcome.last_seq,
        last_payload: outcome.last_payload,
    }))
}

/// The live notices and tool-call metrics a committed projection owes: a
/// subagent's rows ride `subagentProgress`, a top-level Message a
/// `message.upsert`.
async fn notify_projection(
    service: &BackendSandboxServiceImpl,
    task_id: &str,
    harness: Harness,
    projected: &LineProjection,
) {
    for (tool, failed) in &projected.settled {
        let tool = tool.as_deref().map_or("unknown", bucket_tool);
        let status = if *failed { "error" } else { "ok" };
        record_tool_call(tool, Some(harness.as_str()), status);
    }
    let Some(handlers) = service.handlers.as_ref() else {
        return;
    };
    if !projected.skills.is_empty() {
        let tags: Vec<String> = projected
            .skills
            .iter()
            .map(|s| format!("skill-{s}"))
            .collect();
        handlers.tags.propagate_tags(task_id, &tags).await;
    }
    for tool_use_id in &projected.subagents {
        emit_subagent_progress(
            service.events.as_ref(),
            &handlers.messages,
            task_id,
            tool_use_id,
        )
        .await;
    }
    for uuid in &projected.messages {
        emit_message_upsert_from_daos(
            service.events.as_ref(),
            &handlers.messages,
            Some(&handlers.files),
            task_id,
            uuid,
        )
        .await;
    }
}

pub(super) async fn read_trajectory(
    service: &BackendSandboxServiceImpl,
    request: Request<ReadTrajectoryRequest>,
) -> Result<Response<TrajectoryChunkStream>, Status> {
    let task_id = request.get_ref().task_id.clone();
    service.authorizer.authorize(&request, &task_id).await?;
    let dao = store(service)?;
    let req = request.into_inner();
    let harness = harness(&req.harness)?;

    let opened = match req.target {
        Some(read_trajectory_request::Target::HarnessSessionId(id)) => dao
            .find_session(&id)
            .await
            .map_err(|e| internal(e, "Failed to read trajectory session"))?
            .filter(|s| s.task_id == task_id && s.harness == harness.as_str())
            .map(|session| OpenedSession {
                session,
                transcode: Vec::new(),
            }),
        Some(read_trajectory_request::Target::Fork(true)) => {
            fork_session(service, dao, &task_id, harness).await?
        }
        Some(read_trajectory_request::Target::Switch(true)) => dao
            .open_switch_session(&task_id, harness)
            .await
            .map_err(|e| internal(e, "Failed to open the switch session"))?,
        _ => return Err(Status::invalid_argument("a target is required")),
    };
    let Some(OpenedSession { session, transcode }) = opened else {
        return Err(Status::not_found("no trajectory session"));
    };

    let lines = dao.chain(&session).await.map_err(|e| match e {
        ChainError::Gap { .. } => Status::data_loss(e.to_string()),
        ChainError::Db(e) => internal(e, "Failed to read trajectory"),
    })?;
    let header = ReadTrajectoryChunk {
        body: Some(read_trajectory_chunk::Body::Header(SessionHeader {
            harness_session_id: session.harness_session_id,
            line_count: lines.len() as i64,
        })),
    };
    let lines = lines.into_iter().map(|line| ReadTrajectoryChunk {
        body: Some(read_trajectory_chunk::Body::Line(line)),
    });
    let transcode = transcode.into_iter().map(|item| ReadTrajectoryChunk {
        body: Some(read_trajectory_chunk::Body::Transcode(TranscodeItem {
            content_json: item.content.to_string(),
            turn_number: item.turn_number,
            source_harness: item.source_harness,
        })),
    });
    let chunks = std::iter::once(header).chain(lines).chain(transcode);
    Ok(Response::new(Box::pin(futures::stream::iter(
        chunks.map(Ok::<_, Status>),
    ))))
}

/// The session fork `task_id` resumes, created from its `task_forks` row on
/// the first call: verbatim when the cut is in a session of `harness`, else
/// transcoded from it. `None` when the Task is no fork or its cut is not in
/// the source's trajectory; the sandbox then takes the archive path.
async fn fork_session(
    service: &BackendSandboxServiceImpl,
    dao: &Trajectories,
    task_id: &str,
    harness: Harness,
) -> Result<Option<OpenedSession>, Status> {
    let Some(handlers) = service.handlers.as_ref() else {
        return Ok(None);
    };
    let Some(fork) = handlers
        .task_forks
        .find(task_id)
        .await
        .map_err(|e| internal(e, "Failed to read the fork row"))?
    else {
        return Ok(None);
    };
    dao.open_fork_session(task_id, harness, fork.source_event_id)
        .await
        .map_err(|e| internal(e, "Failed to create the fork session"))
}
