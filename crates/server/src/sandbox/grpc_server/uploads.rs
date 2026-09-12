//! An uploaded file, served whole or as the chunks the sandbox reassembles.

use std::pin::Pin;

use futures::{Stream, StreamExt};
use tonic::{Request, Response, Status};
use tracing::{debug, warn};

use sandbox_proto::{DownloadUploadRequest, DownloadUploadResponse, UploadChunk};

use moatless_s3 as file_s3;

use super::state::BackendSandboxServiceImpl;

/// The chunk stream [`download_upload_chunked`] answers with.
pub(super) type UploadChunkStream = Pin<Box<dyn Stream<Item = Result<UploadChunk, Status>> + Send>>;

/// An uploaded file read out of S3, ready to be served by either download RPC.
struct LoadedUpload {
    data: Vec<u8>,
    filename: String,
    mime_type: String,
}

/// Split an upload into the chunk sequence `DownloadUploadChunked` streams.
///
/// The first chunk names the file; the rest carry bytes only. A zero-byte
/// upload still yields one chunk, so the filename always reaches the sandbox
/// and an empty stream stays available to mean "protocol violation".
fn upload_chunks(upload: LoadedUpload) -> Vec<UploadChunk> {
    /// 1 MiB, matching the transcript stream. Small enough to sit well inside
    /// any default per-message cap, large enough that a 50 MiB upload is tens
    /// of messages rather than thousands.
    const CHUNK_SIZE: usize = 1024 * 1024;

    let mut chunks: Vec<UploadChunk> = upload
        .data
        .chunks(CHUNK_SIZE)
        .map(|c| UploadChunk {
            data: c.to_vec(),
            filename: String::new(),
            mime_type: String::new(),
        })
        .collect();

    if chunks.is_empty() {
        chunks.push(UploadChunk {
            data: Vec::new(),
            filename: String::new(),
            mime_type: String::new(),
        });
    }

    if let Some(first) = chunks.first_mut() {
        first.filename = upload.filename;
        first.mime_type = upload.mime_type;
    }

    chunks
}

impl BackendSandboxServiceImpl {
    /// Read an uploaded file's bytes and metadata for the download RPCs.
    ///
    /// Defense in depth on top of the caller's JWT check: the `file_id` must be
    /// referenced by a message belonging to `task_id`, so even a valid token
    /// cannot pull another task's uploads.
    async fn load_upload(&self, task_id: &str, file_id: &str) -> Result<LoadedUpload, Status> {
        let Some(ref s3_client) = self.s3 else {
            return Err(Status::failed_precondition("S3 not configured"));
        };
        let Some(ref bucket) = self.s3_bucket else {
            return Err(Status::failed_precondition("S3 bucket not configured"));
        };
        let Some(files) = self.handlers.as_ref().map(|h| &h.files) else {
            return Err(Status::failed_precondition(
                "File repository not configured",
            ));
        };
        let Some(messages) = self.handlers.as_ref().map(|h| &h.messages) else {
            return Err(Status::failed_precondition(
                "Message repository not configured",
            ));
        };

        let used = messages
            .file_id_used_in_task(task_id, file_id)
            .await
            .map_err(|e| {
                warn!(error = %e, task_id, file_id, "file_id_used_in_task query failed");
                Status::internal("Failed to check file scope")
            })?;
        if !used {
            return Err(Status::not_found("File not attached to this task"));
        }

        let meta = files
            .metadata(file_id)
            .await
            .map_err(|e| {
                warn!(error = %e, file_id, "file metadata lookup failed");
                Status::internal("Failed to look up file metadata")
            })?
            .ok_or_else(|| Status::not_found("File metadata not found"))?;

        let s3_key = format!("{}/{}", self.s3_key_prefix, file_id);
        let (data, _content_type) = file_s3::get_object(&s3_client.client, bucket, &s3_key)
            .await
            .map_err(|e| {
                warn!(error = %e, file_id, "S3 get_object failed");
                Status::internal("Failed to fetch file from S3")
            })?;

        debug!(
            task_id,
            file_id,
            bytes = data.len(),
            "Served file upload to sandbox"
        );

        Ok(LoadedUpload {
            data,
            filename: meta.filename,
            mime_type: meta.mime_type,
        })
    }
}

/// Serve an uploaded file's bytes to the sandbox in one message.
///
/// Bounded by the transport's per-message cap. Auth and scope checks are
/// [`BackendSandboxServiceImpl::load_upload`]'s.
pub(super) async fn download_upload(
    service: &BackendSandboxServiceImpl,
    request: Request<DownloadUploadRequest>,
) -> Result<Response<DownloadUploadResponse>, Status> {
    let task_id = request.get_ref().task_id.clone();
    service.authorizer.authorize(&request, &task_id).await?;
    let req = request.into_inner();

    let upload = service.load_upload(&req.task_id, &req.file_id).await?;

    Ok(Response::new(DownloadUploadResponse {
        data: upload.data,
        filename: upload.filename,
        mime_type: upload.mime_type,
    }))
}

/// Stream an uploaded file's bytes to the sandbox, chunk by chunk.
///
/// The sandbox calls this for every `file_id` carried on a pending message
/// before invoking the agent, so the on-disk path advertised in the prompt
/// ("Saved to: …") is valid by the time the agent reads it. Chunking is
/// what keeps an upload's size off the wire's critical path: each message
/// is `CHUNK_SIZE`, whatever the file weighs.
///
/// The object is read from S3 before the first chunk goes out, so the
/// client-side call timeout — which tonic applies to the response headers
/// and not to the body that follows — covers the S3 read.
///
/// Auth and scope checks are [`BackendSandboxServiceImpl::load_upload`]'s.
pub(super) async fn download_upload_chunked(
    service: &BackendSandboxServiceImpl,
    request: Request<DownloadUploadRequest>,
) -> Result<Response<UploadChunkStream>, Status> {
    let task_id = request.get_ref().task_id.clone();
    service.authorizer.authorize(&request, &task_id).await?;
    let req = request.into_inner();

    let upload = service.load_upload(&req.task_id, &req.file_id).await?;
    let chunks = upload_chunks(upload);

    let stream = futures::stream::iter(chunks).map(Ok::<UploadChunk, Status>);
    Ok(Response::new(Box::pin(stream)))
}

#[cfg(test)]
mod tests {
    use super::*;

    // DownloadUploadChunked — the chunk sequence the sandbox reassembles

    /// The chunk size `upload_chunks` splits on. Kept in the test as a literal
    /// so a change to the constant has to be restated here deliberately.
    const CHUNK: usize = 1024 * 1024;

    fn upload_of(len: usize) -> LoadedUpload {
        LoadedUpload {
            // A position-dependent byte pattern, so a test that reassembles the
            // chunks catches reordering and duplication, not just a length slip.
            data: (0..len)
                .map(|i| u8::try_from(i % 251).unwrap_or(0))
                .collect(),
            filename: "report.pdf".to_string(),
            mime_type: "application/pdf".to_string(),
        }
    }

    /// Concatenate the chunk payloads back into the file the sandbox writes.
    fn reassemble(chunks: &[UploadChunk]) -> Vec<u8> {
        chunks.iter().flat_map(|c| c.data.clone()).collect()
    }

    #[test]
    fn only_the_first_chunk_names_the_file() {
        let chunks = upload_chunks(upload_of(3 * CHUNK));
        let (first, rest) = chunks.split_first().expect("at least one chunk");
        assert_eq!(first.filename, "report.pdf");
        assert_eq!(first.mime_type, "application/pdf");
        for chunk in rest {
            assert!(
                chunk.filename.is_empty(),
                "only the first chunk names the file"
            );
            assert!(chunk.mime_type.is_empty());
        }
    }

    #[test]
    fn an_upload_larger_than_the_unary_cap_survives_chunking() {
        // 5 MiB: past tonic's 4 MiB default per-message cap, which is what the
        // single-message RPC could not carry.
        let len = 5 * CHUNK;
        let chunks = upload_chunks(upload_of(len));

        assert_eq!(chunks.len(), 5);
        for chunk in &chunks {
            assert!(
                chunk.data.len() <= CHUNK,
                "every message must stay under the per-message cap"
            );
        }
        assert_eq!(reassemble(&chunks), upload_of(len).data);
    }

    #[test]
    fn an_empty_upload_is_one_chunk_carrying_only_the_name() {
        // Not an empty stream: the sandbox needs the filename to know what to
        // write, and reserves "no chunks" for a server that failed to speak.
        let chunks = upload_chunks(upload_of(0));

        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].data.is_empty());
        assert_eq!(chunks[0].filename, "report.pdf");
    }

    #[test]
    fn a_chunk_sized_upload_does_not_produce_a_trailing_empty_chunk() {
        for len in [CHUNK, 2 * CHUNK] {
            let chunks = upload_chunks(upload_of(len));
            assert_eq!(
                chunks.len(),
                len / CHUNK,
                "exact multiple of the chunk size"
            );
            assert!(
                chunks.iter().all(|c| c.data.len() == CHUNK),
                "no short chunk when the file divides evenly"
            );
            assert_eq!(reassemble(&chunks), upload_of(len).data);
        }
    }

    #[test]
    fn an_upload_one_byte_past_a_boundary_spills_into_a_short_chunk() {
        let chunks = upload_chunks(upload_of(CHUNK + 1));

        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].data.len(), CHUNK);
        assert_eq!(chunks[1].data.len(), 1);
        assert_eq!(reassemble(&chunks), upload_of(CHUNK + 1).data);
    }
}
