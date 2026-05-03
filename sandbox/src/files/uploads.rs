//! Pre-fetch user-uploaded files referenced by pending messages.
//!
//! When a user attaches files to a message, the backend persists their
//! `file_id`s on the message and advertises an on-disk path (under
//! `UPLOAD_DIR`) in the prompt. This module fetches the bytes via gRPC and
//! writes them to that path before the agent runs, so the prompt is valid
//! by the time the agent reads it.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::{debug, warn};

use crate::backend_client::BackendClient;

/// Errors from pre-fetching uploaded files into the sandbox filesystem.
///
/// `tonic::Status` is large, so it's boxed to keep this enum cheap to pass
/// around (clippy `result_large_err`).
#[derive(Debug, thiserror::Error)]
pub enum UploadFetchError {
    #[error("gRPC download failed: {0}")]
    Grpc(Box<tonic::Status>),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Filename is empty")]
    EmptyFilename,
}

impl From<tonic::Status> for UploadFetchError {
    fn from(s: tonic::Status) -> Self {
        Self::Grpc(Box::new(s))
    }
}

/// Ensure an uploaded file is on disk under `<upload_dir>/<file_id>/<filename>`.
///
/// Idempotent: if the target directory already contains files we assume a
/// previous turn (or pod restart with PVC) already wrote them and return
/// without making the gRPC call.
pub async fn ensure_upload_on_disk(
    backend: &Arc<dyn BackendClient>,
    task_id: &str,
    file_id: &str,
    upload_dir: &str,
) -> Result<PathBuf, UploadFetchError> {
    let target_dir = Path::new(upload_dir).join(file_id);

    if dir_already_populated(&target_dir).await {
        debug!(
            file_id,
            dir = %target_dir.display(),
            "Upload already on disk, skipping fetch"
        );
        return Ok(target_dir);
    }

    let resp = backend.download_upload(task_id, file_id).await?;

    let safe_name = sanitize_filename(&resp.filename)?;
    tokio::fs::create_dir_all(&target_dir).await?;
    let file_path = target_dir.join(&safe_name);
    tokio::fs::write(&file_path, &resp.data).await?;

    debug!(
        file_id,
        path = %file_path.display(),
        bytes = resp.data.len(),
        "Wrote uploaded file to disk"
    );

    Ok(target_dir)
}

/// Best-effort batch fetch — logs failures and continues. The agent will
/// see a missing path if a fetch fails and can report it back to the user.
pub async fn fetch_uploads_for_messages(
    backend: &Arc<dyn BackendClient>,
    task_id: &str,
    file_ids: impl IntoIterator<Item = String>,
    upload_dir: &str,
) {
    for file_id in file_ids {
        if let Err(e) = ensure_upload_on_disk(backend, task_id, &file_id, upload_dir).await {
            warn!(error = %e, file_id, "Failed to fetch attached upload");
        }
    }
}

/// Strip path separators from an upload's user-supplied filename. Backend
/// records arbitrary user input; we only need a leaf name on disk.
fn sanitize_filename(filename: &str) -> Result<String, UploadFetchError> {
    let trimmed = filename.trim();
    if trimmed.is_empty() {
        return Err(UploadFetchError::EmptyFilename);
    }
    let leaf = Path::new(trimmed)
        .file_name()
        .and_then(|s| s.to_str())
        .map(std::string::ToString::to_string)
        .filter(|s| !s.is_empty() && s != "." && s != "..");

    Ok(leaf.unwrap_or_else(|| "upload.bin".to_string()))
}

async fn dir_already_populated(dir: &Path) -> bool {
    let mut entries = match tokio::fs::read_dir(dir).await {
        Ok(e) => e,
        Err(_) => return false,
    };
    matches!(entries.next_entry().await, Ok(Some(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_filename_strips_path_separators() {
        assert_eq!(sanitize_filename("foo.json").unwrap(), "foo.json");
        assert_eq!(sanitize_filename("../foo.json").unwrap(), "foo.json");
        assert_eq!(sanitize_filename("/etc/passwd").unwrap(), "passwd");
        assert_eq!(sanitize_filename("dir/sub/file.txt").unwrap(), "file.txt");
    }

    #[test]
    fn sanitize_filename_rejects_empty() {
        assert!(matches!(
            sanitize_filename(""),
            Err(UploadFetchError::EmptyFilename)
        ));
        assert!(matches!(
            sanitize_filename("   "),
            Err(UploadFetchError::EmptyFilename)
        ));
    }

    #[test]
    fn sanitize_filename_handles_dots() {
        // "." and ".." resolve to no valid leaf — fall back to default.
        assert_eq!(sanitize_filename(".").unwrap(), "upload.bin");
        assert_eq!(sanitize_filename("..").unwrap(), "upload.bin");
    }
}
