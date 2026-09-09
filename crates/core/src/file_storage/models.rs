//! What an upload is to a caller, and the row it is stored as.

use chrono::{DateTime, Utc};

use moatless_types::access_control::Scope;

/// Row from the `file_metadata` table.
#[derive(sqlx::FromRow, Debug, Clone)]
pub(crate) struct FileMetadata {
    pub file_id: String,
    pub filename: String,
    pub mime_type: String,
    pub size: i64,
    #[sqlx(try_from = "String")]
    pub scope: Scope,
    pub owner_user_id: Option<String>,
    pub created_by: Option<String>,
    pub uploaded_at: DateTime<Utc>,
}

impl FileMetadata {
    pub(crate) fn info(&self) -> FileInfo {
        FileInfo {
            file_id: self.file_id.clone(),
            filename: self.filename.clone(),
            mime_type: self.mime_type.clone(),
            size: self.size,
        }
    }
}

/// One file's bytes, and what the sender called them.
pub struct NewUpload {
    pub filename: String,
    pub mime_type: String,
    pub data: Vec<u8>,
    /// The id to store under, when the caller already handed one out. The T3
    /// attachment flow mints the id before the bytes move — the client holds it
    /// while the upload is in flight and names it on the turn afterwards — so
    /// that id has to be the one the row is written under. `None` mints a fresh
    /// uuid here, which is what both of the other two surfaces want.
    pub file_id: Option<String>,
}

/// An upload as every caller that is not reading its bytes sees it.
#[derive(Debug, Clone)]
pub struct FileInfo {
    pub file_id: String,
    pub filename: String,
    pub mime_type: String,
    pub size: i64,
}

/// File bytes and the metadata needed by the HTTP download adapter.
pub struct Download {
    pub data: Vec<u8>,
    pub content_type: String,
    pub filename: String,
}

/// An upload resolved for an outbound message.
pub struct Attachment {
    pub filename: String,
    pub mime_type: String,
    pub data: Vec<u8>,
}
