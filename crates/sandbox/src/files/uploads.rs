//! Pre-fetch user-uploaded files referenced by pending messages.
//!
//! The backend advertises an on-disk path under `UPLOAD_DIR` in the prompt
//! before the bytes exist; this module fetches them via gRPC so the path is
//! valid by the time the agent reads it.
//!
//! `UPLOAD_DIR` lives under `WORKSPACE_DIR` and rides the workspace snapshot —
//! see [`ensure_upload_on_disk`] for the restart-idempotent behavior it backs.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::{debug, error};

use crate::backend_client::BackendClient;

/// Errors from pre-fetching uploaded files into the sandbox filesystem.
///
/// `tonic::Status` is boxed to keep this enum cheap to pass around (clippy
/// `result_large_err`).
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

/// Ensure an uploaded file is on disk under
/// `<upload_dir>/<file_id>/<filename>`.
///
/// Idempotent: skips the gRPC call when the target directory is already
/// populated.
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

/// Batch fetch — a failure on one file stops neither the others nor the turn.
///
/// Returns the failed `file_id`s paired with why, so the caller can correct a
/// prompt that would otherwise advertise an unwritten path as valid.
pub async fn fetch_uploads_for_messages(
    backend: &Arc<dyn BackendClient>,
    task_id: &str,
    file_ids: impl IntoIterator<Item = String>,
    upload_dir: &str,
) -> Vec<(String, String)> {
    let mut failed = Vec::new();
    for file_id in file_ids {
        if let Err(e) = ensure_upload_on_disk(backend, task_id, &file_id, upload_dir).await {
            error!(error = %e, file_id, "Failed to fetch attached upload");
            failed.push((file_id, e.to_string()));
        }
    }
    failed
}

/// Render the note appended to a turn's prompt when an attachment did not reach
/// the sandbox.
///
/// Names each attachment by `file_id`, matching the prompt's "Saved to:" line
/// it contradicts. Returns `None` when every fetch succeeded.
pub fn failed_upload_note(failed: &[(String, String)]) -> Option<String> {
    if failed.is_empty() {
        return None;
    }

    let lines = failed
        .iter()
        .fold(String::new(), |mut acc, (file_id, error)| {
            use std::fmt::Write as _;
            let _ = writeln!(acc, "- {file_id}: {error}");
            acc
        });

    Some(format!(
        "\n\n[system] These attached files could NOT be transferred into this sandbox. \
         Nothing was written to the \"Saved to:\" paths above for them, so do not read \
         those paths or claim to have read the files — tell the user the transfer failed:\n\
         {lines}"
    ))
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

    #[test]
    fn a_turn_whose_uploads_all_landed_gets_no_note() {
        assert!(failed_upload_note(&[]).is_none());
    }

    #[test]
    fn the_note_names_every_file_that_did_not_land() {
        let note = failed_upload_note(&[
            (
                "file-a".to_string(),
                "gRPC download failed: unavailable".to_string(),
            ),
            ("file-b".to_string(), "IO error: no space left".to_string()),
        ])
        .expect("failures must produce a note");

        // The file_id is how the agent matches the note to the prompt's
        // "Saved to: <dir>/<file_id>/<name>" line.
        assert!(note.contains("file-a"), "{note}");
        assert!(note.contains("file-b"), "{note}");
        assert!(note.contains("no space left"), "{note}");
        assert!(
            note.contains("could NOT be transferred"),
            "the note must contradict the Saved to: line, not merely annotate it: {note}"
        );
    }
}
