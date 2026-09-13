//! The transcript tar a sandbox stores, and the one a fork starts from.

use std::pin::Pin;

use futures::{Stream, StreamExt};
use tonic::{Request, Response, Status, Streaming};
use tracing::{debug, warn};

use sandbox_proto::{DownloadTranscriptRequest, TranscriptChunk, UploadTranscriptResponse};

use moatless_s3 as file_s3;
use moatless_tasks::task::relations::TaskRelation;
use moatless_types::task::TaskRelationType;

use super::state::BackendSandboxServiceImpl;

/// The chunk stream [`download_transcript`] answers with.
pub(super) type TranscriptChunkStream =
    Pin<Box<dyn Stream<Item = Result<TranscriptChunk, Status>> + Send>>;

/// The Task whose archives a fork falls back to, read off its parent edge.
///
/// Keyed on `forked_from`, not `task_forks` (written only with session
/// anchors), so a fork whose parent never ran an agent still resolves; `None`
/// for no parent or a `created_by` parent, since a subtask must not restore its
/// spawner's tree.
pub(super) fn fork_source_task_id(parent: Option<&TaskRelation>) -> Option<&str> {
    let row = parent?;
    (row.relation_type == TaskRelationType::ForkedFrom).then_some(row.to_task_id.as_str())
}

/// Build the S3 key for a task's transcript archive:
/// `<prefix>/<task_id>/transcripts.tar`. The `<prefix>/<task_id>/` portion is
/// the same one the Kubernetes runtime removes, so uploads and cleanup stay
/// aligned.
fn transcript_key(prefix: &str, task_id: &str) -> String {
    format!(
        "{}/{}/transcripts.tar",
        prefix.trim_end_matches('/'),
        task_id
    )
}

impl BackendSandboxServiceImpl {
    /// The transcript archive of the Task a fork branched from.
    ///
    /// Empty when not a fork, the source archive is missing, or the lookup
    /// fails — the pod then starts fresh. Called only when the Task has no
    /// archive of its own, so it never displaces a fork's own transcript.
    async fn source_transcript(
        &self,
        s3_client: &moatless_s3::S3Storage,
        bucket: &str,
        task_id: &str,
    ) -> Vec<u8> {
        let Some(handlers) = self.handlers.as_ref() else {
            return Vec::new();
        };
        let parent = match handlers.task_relations.find_parent(task_id).await {
            Ok(parent) => parent,
            Err(e) => {
                warn!(error = %e, task_id = %task_id, "Failed to read the fork relation; restoring no transcript");
                return Vec::new();
            }
        };
        // The parent edge, not `task_forks`: a rewind overwrites that row's
        // `source_task_id` with the Task's own, so a rewound fork would ask for
        // its own missing archive. See [`fork_source_task_id`].
        let Some(source_task_id) = fork_source_task_id(parent.as_ref()).map(str::to_owned) else {
            return Vec::new();
        };

        let key = transcript_key(&self.transcript_s3_prefix, &source_task_id);
        match file_s3::get_object_opt(&s3_client.client, bucket, &key).await {
            Ok(Some(bytes)) => {
                debug!(
                    task_id = %task_id,
                    source_task_id = %source_task_id,
                    bytes = bytes.len(),
                    "Serving the source task's transcript to a fork that has none of its own"
                );
                bytes
            }
            Ok(None) => Vec::new(),
            Err(e) => {
                warn!(error = %e, task_id = %task_id, source_task_id = %source_task_id, "S3 get_object for the source task's transcript failed");
                Vec::new()
            }
        }
    }
}

/// Receive a streamed transcript tar from the sandbox and store it in S3.
///
/// The sandbox is AWS-credential-less, so it proxies all transcript I/O through
/// here. Client-streaming, so a transcript bigger than tonic's unary cap still
/// round-trips; chunks are reassembled in memory and `put_object`ed, with
/// `task_id` taken from the first chunk.
pub(super) async fn upload_transcript(
    service: &BackendSandboxServiceImpl,
    request: Request<Streaming<TranscriptChunk>>,
) -> Result<Response<UploadTranscriptResponse>, Status> {
    // Verify the token up front (from metadata), but defer the task grant:
    // the task_id arrives in-band from the stream below.
    let verified = service.authorizer.verify_token(&request, false)?;

    let Some(ref s3_client) = service.s3 else {
        return Err(Status::failed_precondition("S3 not configured"));
    };
    let Some(ref bucket) = service.s3_bucket else {
        return Err(Status::failed_precondition("S3 bucket not configured"));
    };

    let mut stream = request.into_inner();
    let mut task_id = String::new();
    let mut data: Vec<u8> = Vec::new();
    while let Some(chunk) = stream.message().await? {
        if task_id.is_empty() && !chunk.task_id.is_empty() {
            task_id = chunk.task_id;
        }
        data.extend_from_slice(&chunk.data);
    }

    if task_id.is_empty() {
        return Err(Status::invalid_argument(
            "task_id missing from transcript stream",
        ));
    }

    service.authorizer.resolve_grant(verified, &task_id).await?;

    let bytes_received = data.len() as i64;
    let key = transcript_key(&service.transcript_s3_prefix, &task_id);
    file_s3::put_object(&s3_client.client, bucket, &key, data, "application/x-tar")
        .await
        .map_err(|e| {
            warn!(error = %e, task_id = %task_id, "S3 put_object for transcript failed");
            Status::internal("Failed to store transcript in S3")
        })?;

    debug!(task_id = %task_id, bytes = bytes_received, key = %key, "Stored transcript in S3");
    Ok(Response::new(UploadTranscriptResponse { bytes_received }))
}

/// Stream the task's stored transcript tar back to the sandbox.
///
/// An empty stream means nothing stored yet (the sandbox reads it as "nothing
/// to restore"); a fork with no archive of its own falls back to its branch
/// parent's, the only way a resumed session reaches an isolated fork's fresh
/// pod.
pub(super) async fn download_transcript(
    service: &BackendSandboxServiceImpl,
    request: Request<DownloadTranscriptRequest>,
) -> Result<Response<TranscriptChunkStream>, Status> {
    let task_id = request.get_ref().task_id.clone();
    service.authorizer.authorize(&request, &task_id).await?;
    let req = request.into_inner();

    let Some(ref s3_client) = service.s3 else {
        return Err(Status::failed_precondition("S3 not configured"));
    };
    let Some(ref bucket) = service.s3_bucket else {
        return Err(Status::failed_precondition("S3 bucket not configured"));
    };

    let key = transcript_key(&service.transcript_s3_prefix, &req.task_id);
    let own = file_s3::get_object_opt(&s3_client.client, bucket, &key)
        .await
        .map_err(|e| {
            warn!(error = %e, task_id = %req.task_id, "S3 get_object for transcript failed");
            Status::internal("Failed to fetch transcript from S3")
        })?
        .filter(|bytes| !bytes.is_empty());

    // A fork that has not run yet has no archive of its own, and the session
    // its first turn resumes is the parent's. Falling back only when the fork's
    // own object is absent is what keeps a restart after that first turn on its
    // own transcript rather than back on the parent's.
    let data = match own {
        Some(bytes) => bytes,
        None => {
            service
                .source_transcript(s3_client, bucket, &req.task_id)
                .await
        }
    };

    debug!(task_id = %req.task_id, bytes = data.len(), "Serving transcript to sandbox");

    // Chunk the in-memory tar so each gRPC message stays well under the default
    // limit. An empty tar yields no chunks → an empty stream.
    const CHUNK_SIZE: usize = 1024 * 1024; // 1 MiB
    let chunks: Vec<TranscriptChunk> = data
        .chunks(CHUNK_SIZE)
        .map(|c| TranscriptChunk {
            task_id: String::new(),
            data: c.to_vec(),
        })
        .collect();

    let stream = futures::stream::iter(chunks).map(Ok::<TranscriptChunk, Status>);
    Ok(Response::new(Box::pin(stream)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parent_edge(relation_type: TaskRelationType) -> TaskRelation {
        TaskRelation::new("fork", "source", relation_type, serde_json::Value::Null)
    }

    #[test]
    fn a_forked_from_parent_is_the_archive_source() {
        let edge = parent_edge(TaskRelationType::ForkedFrom);
        assert_eq!(fork_source_task_id(Some(&edge)), Some("source"));
    }

    /// `find_parent` answers `created_by` as well, and a subtask restoring the
    /// tree of the Task that spawned it would overwrite its own checkout.
    #[test]
    fn a_created_by_parent_is_not_an_archive_source() {
        let edge = parent_edge(TaskRelationType::CreatedBy);
        assert_eq!(fork_source_task_id(Some(&edge)), None);
    }

    #[test]
    fn a_task_with_no_parent_has_no_archive_source() {
        assert_eq!(fork_source_task_id(None), None);
    }
}
