//! [`AttachmentStore`], and the half of [`AppStateCommands`] that turns a
//! turn's attachments into the upload ids its Message will name.

use super::commands::AppStateCommands;
use crate::state::AppState;
use async_trait::async_trait;
use moatless_core::file_storage::FileStorageError;
use moatless_core::file_storage::NewUpload;
use moatless_t3code::ports::{
    AttachmentKind, AttachmentStore, AttachmentStoreError, CommandRefusal, DeclaredAttachment,
    TurnAttachment,
};
use moatless_types::access_control::ScopeContext;

/// A turn's attachments as upload ids, and which of those this call stored.
pub(super) struct ResolvedAttachments {
    /// Every id the Message will name, in the order they were attached.
    pub(super) file_ids: Vec<String>,
    /// The subset stored here from inline bytes, and so the only ids a failed
    /// command may take back.
    ///
    /// An `Uploaded` id came from the client via `attachments.createUploadUrl`;
    /// the client still holds it and re-sends it on retry, so deleting it would
    /// break that retry.
    pub(super) stored_here: Vec<String>,
}

impl AppStateCommands {
    /// The upload ids a turn's attachments name, storing the ones that
    /// arrive as bytes.
    ///
    /// Both forms end as a Moatless upload id: an inline image is stored
    /// here, an uploaded attachment is only checked (not re-stored). A
    /// failure refuses the turn and takes back any bytes already stored.
    pub(super) async fn resolve_attachments(
        &self,
        viewer: &ScopeContext,
        attachments: &[TurnAttachment],
    ) -> Result<ResolvedAttachments, CommandRefusal> {
        let mut resolved = ResolvedAttachments {
            file_ids: Vec::with_capacity(attachments.len()),
            stored_here: Vec::new(),
        };
        for attachment in attachments {
            let outcome = match attachment {
                TurnAttachment::Inline {
                    filename,
                    mime_type,
                    bytes,
                } => {
                    let upload = NewUpload {
                        filename: filename.clone(),
                        mime_type: mime_type.clone(),
                        data: bytes.clone(),
                        file_id: None,
                    };
                    self.0
                        .files
                        .store_upload(viewer, upload)
                        .await
                        .map_err(|error| {
                            tracing::warn!(%error, filename = %filename, "T3 attachment could not be stored");
                            CommandRefusal::Failed {
                                message: format!("Could not store the attachment `{filename}`."),
                            }
                        })
                        .map(|stored| {
                            resolved.stored_here.push(stored.file_id.clone());
                            stored.file_id
                        })
                }
                TurnAttachment::Uploaded {
                    attachment_id,
                    kind,
                } => self.uploaded_attachment(viewer, attachment_id, *kind).await,
            };
            match outcome {
                Ok(file_id) => resolved.file_ids.push(file_id),
                Err(refusal) => {
                    self.discard_attachments(viewer, &resolved.stored_here)
                        .await;
                    return Err(refusal);
                }
            }
        }
        Ok(resolved)
    }

    /// The upload an attachment id names, if this viewer may read it.
    ///
    /// Checked rather than trusted: nothing about a turn proves its sender
    /// minted this id, so a nonexistent id and someone else's upload get the
    /// same answer — otherwise a turn could probe whether an upload exists.
    async fn uploaded_attachment(
        &self,
        viewer: &ScopeContext,
        attachment_id: &str,
        kind: AttachmentKind,
    ) -> Result<String, CommandRefusal> {
        let unreachable = || CommandRefusal::Rejected {
            message: "An attachment on this message is no longer available.".to_string(),
        };
        let meta = match self.0.files.readable_metadata(viewer, attachment_id).await {
            Ok(Some(meta)) => meta,
            Ok(None) => return Err(unreachable()),
            Err(FileStorageError::Forbidden(_)) => {
                tracing::warn!(
                    attachment_id,
                    "T3 turn named an attachment its sender cannot read"
                );
                return Err(unreachable());
            }
            Err(error) => {
                tracing::warn!(%error, attachment_id, "T3 attachment could not be resolved");
                return Err(CommandRefusal::Failed {
                    message: "Could not reach that attachment.".to_string(),
                });
            }
        };
        // Read access isn't provenance: a file this viewer can read may
        // have bypassed T3's mint via Moatless's multipart route, so this
        // is what holds the mime/size limits. It also matches the
        // projection, which drops an out-of-bounds attachment the agent
        // saw but the client would never render.
        if !kind.accepts(&meta.mime_type, meta.size) {
            tracing::warn!(
                attachment_id,
                mime_type = %meta.mime_type,
                size = meta.size,
                ?kind,
                "T3 turn named an attachment outside the member it claimed"
            );
            return Err(CommandRefusal::Rejected {
                message: format!(
                    "Attachment `{}` is not something that can be sent here.",
                    meta.filename
                ),
            });
        }
        Ok(meta.file_id)
    }

    /// Take back inline uploads a failed command left behind.
    ///
    /// Only passed [`ResolvedAttachments::stored_here`]: no Message was ever
    /// written, so no [`AppStateAttachments::is_on_a_message`] check applies
    /// here, unlike `attachments.delete`. Best-effort: an unremovable store
    /// is logged, not raised as a second failure.
    pub(super) async fn discard_attachments(&self, viewer: &ScopeContext, file_ids: &[String]) {
        for file_id in file_ids {
            if let Err(error) = self.0.files.delete(viewer, file_id).await {
                tracing::warn!(%error, file_id, "T3 attachment outlived the command it was sent with");
            }
        }
    }
}

/// Where a T3 chat attachment's bytes go: an ordinary Moatless upload, under
/// the id `attachments.createUploadUrl` already handed the client.
pub struct AppStateAttachments(pub AppState);

impl AppStateAttachments {
    /// Whether a sent message already carries this upload.
    ///
    /// Lives here rather than in `Files::delete`, which is generic storage with
    /// no notion of messages: the rule that an attachment stops being a draft
    /// once a turn names it belongs to this surface.
    async fn is_on_a_message(&self, attachment_id: &str) -> Result<bool, AttachmentStoreError> {
        self.0.messages.file_on_any_message(attachment_id)
        .await
        .map_err(|error| {
            // Not knowing is not permission to delete: a read that failed is
            // reported as a failure rather than assumed to mean "unreferenced".
            tracing::warn!(%error, attachment_id, "Could not tell whether a T3 attachment is in use");
            AttachmentStoreError::Failed(error.to_string())
        })
    }
}

#[async_trait]
impl AttachmentStore for AppStateAttachments {
    async fn store(
        &self,
        viewer: &ScopeContext,
        attachment_id: &str,
        declared: DeclaredAttachment,
        bytes: Vec<u8>,
    ) -> Result<(), AttachmentStoreError> {
        // Once a turn carries this id, the bytes are what the transcript, the
        // download route and the Sandbox pre-fetch all point at, and replacing
        // them would change an attachment that has already been sent and read.
        // A retry cannot reach this: sending the turn required the first upload
        // to have finished.
        if self.is_on_a_message(attachment_id).await? {
            tracing::warn!(
                attachment_id,
                "Refused to replace a T3 attachment already on a message"
            );
            return Err(AttachmentStoreError::Forbidden);
        }

        // A minted URL is good for ten minutes and a client that lost its
        // connection mid-upload may `POST` it again. Removing what the first
        // attempt left makes the second one land instead of colliding with it.
        // Nothing else can be under this id: it is a uuid minted for this
        // viewer and handed to nobody else.
        if let Err(error) = self.0.files.delete(viewer, attachment_id).await {
            tracing::debug!(%error, attachment_id, "T3 attachment had nothing to replace");
        }

        self.0
            .files
            .store_upload(
                viewer,
                NewUpload {
                    filename: declared.filename,
                    mime_type: declared.mime_type,
                    data: bytes,
                    file_id: Some(attachment_id.to_string()),
                },
            )
            .await
            .map(|_| ())
            .map_err(attachment_store_error)
    }

    async fn delete(
        &self,
        viewer: &ScopeContext,
        attachment_id: &str,
    ) -> Result<(), AttachmentStoreError> {
        // `attachments.delete` means "I abandoned this upload" — not true
        // once a message names it, so deleting anyway would leave the
        // transcript, download and Sandbox pre-fetch pointing at gone
        // bytes. Reports success regardless: the client's "nothing
        // dangling" intent holds, and a failure here isn't actionable.
        if self.is_on_a_message(attachment_id).await? {
            tracing::debug!(attachment_id, "Kept a T3 attachment that a message names");
            return Ok(());
        }

        self.0
            .files
            .delete(viewer, attachment_id)
            .await
            .map(|_| ())
            .map_err(attachment_store_error)
    }
}

fn attachment_store_error(error: FileStorageError) -> AttachmentStoreError {
    match error {
        FileStorageError::Forbidden(_) => AttachmentStoreError::Forbidden,
        other => AttachmentStoreError::Failed(other.to_string()),
    }
}
