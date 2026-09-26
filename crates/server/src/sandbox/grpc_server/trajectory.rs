//! `AppendTrajectory` and `ReadTrajectory`: the trajectory a sandbox records
//! its harness session file into, and writes it back out from.

use moatless_types::agent::Harness;
use std::pin::Pin;

use futures::Stream;
use tonic::{Request, Response, Status};
use tracing::warn;

use sandbox_proto::{
    AppendTrajectoryRequest, AppendTrajectoryResponse, ReadTrajectoryChunk, ReadTrajectoryRequest,
    SessionHeader, read_trajectory_chunk, read_trajectory_request,
};

use moatless_tasks::trajectory::models::{
    AppendError, AppendRequest, ChainError, ForkCut, NewEvent, SubagentParent, TrajectorySession,
};
use moatless_tasks::trajectory::operations::Trajectories;

use super::state::BackendSandboxServiceImpl;

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
        })
        .await
        .map_err(|e| match e {
            AppendError::Foreign(_) => Status::permission_denied(e.to_string()),
            AppendError::ParentMissing(_) => Status::failed_precondition(e.to_string()),
            AppendError::Db(e) => internal(e, "Failed to append trajectory"),
        })?;
    Ok(Response::new(AppendTrajectoryResponse {
        accepted: outcome.accepted,
        last_seq: outcome.last_seq,
        last_payload: outcome.last_payload,
    }))
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

    let session = match req.target {
        Some(read_trajectory_request::Target::HarnessSessionId(id)) => dao
            .find_session(&id)
            .await
            .map_err(|e| internal(e, "Failed to read trajectory session"))?
            .filter(|s| s.task_id == task_id && s.harness == harness.as_str()),
        Some(read_trajectory_request::Target::Fork(true)) => {
            fork_session(service, dao, &task_id, harness).await?
        }
        _ => return Err(Status::invalid_argument("a target is required")),
    };
    let Some(session) = session else {
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
    let chunks = std::iter::once(header).chain(lines.into_iter().map(|line| ReadTrajectoryChunk {
        body: Some(read_trajectory_chunk::Body::Line(line)),
    }));
    Ok(Response::new(Box::pin(futures::stream::iter(
        chunks.map(Ok::<_, Status>),
    ))))
}

/// The fork session `task_id` resumes, created from its `task_forks` row on
/// the first call. `None` when the Task is no fork, forked another harness, or
/// its cut is not in the source's trajectory; the sandbox then takes the
/// archive path.
async fn fork_session(
    service: &BackendSandboxServiceImpl,
    dao: &Trajectories,
    task_id: &str,
    harness: Harness,
) -> Result<Option<TrajectorySession>, Status> {
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
    let Some(source) = dao
        .find_session(&fork.source_session_id)
        .await
        .map_err(|e| internal(e, "Failed to read trajectory session"))?
    else {
        return Ok(None);
    };
    if source.harness != harness.as_str() {
        return Ok(None);
    }
    let cut = match (fork.source_message_uuid, fork.source_agent_turn_id) {
        (Some(message), _) if harness == Harness::ClaudeCode => ForkCut::Message(message),
        (_, Some(turn)) if harness == Harness::Codex => ForkCut::Turn(turn),
        _ => ForkCut::End,
    };
    let Some(at) = dao
        .resolve_cut(&fork.source_session_id, &cut)
        .await
        .map_err(|e| internal(e, "Failed to resolve the fork cut"))?
    else {
        return Ok(None);
    };
    dao.insert_fork_session(task_id, harness, at)
        .await
        .map(Some)
        .map_err(|e| internal(e, "Failed to create the fork session"))
}
