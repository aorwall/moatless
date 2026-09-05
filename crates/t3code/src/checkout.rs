//! Where a conversation's files are read from: its Sandbox while one is
//! running, and the workspace snapshot S3 holds once none is.
//!
//! One type, so the four read surfaces — the file tree, the picker, the editor
//! and the asset route — make that choice once and in the same terms. A write
//! never comes through here: it needs a Sandbox to land in, which is why
//! [`crate::files::write_file`] takes the connector on its own.

use base64::Engine;
use moatless_ports::sandbox::{SandboxError, SnapshotError, WorkspaceSnapshotSource};
use moatless_types::sandbox::no_live_sandbox;
use sandbox_proto::client::{SandboxClient, SandboxConnector, classify_grpc_error};
use tracing::warn;

use crate::workspace;
use crate::workspace::WorkspaceAccessError;

/// The ports a checkout is opened through, together so a read surface takes one
/// value rather than two positional handles it never looks inside.
///
/// `pub` because [`crate::files::list_entries`] and its siblings take it and are
/// exercised directly by the backend's integration tests.
#[derive(Clone, Copy)]
pub struct Sources<'a> {
    pub sandboxes: &'a dyn SandboxConnector,
    /// Absent when the deployment stores no workspace snapshots — it has no S3
    /// bucket configured — in which case a stopped Task has no files at all.
    pub snapshots: Option<&'a dyn WorkspaceSnapshotSource>,
}

/// One conversation's files, from whichever copy of them can answer.
pub(crate) struct Checkout<'a> {
    task_id: String,
    backing: Backing<'a>,
}

enum Backing<'a> {
    Live(SandboxClient),
    /// The snapshot, plus what the Sandbox said when it could not be reached.
    /// That text is the answer a caller gets if the snapshot turns out to hold
    /// nothing for this Task: the reason there are no files is still that
    /// nothing is running.
    Stored {
        source: &'a dyn WorkspaceSnapshotSource,
        unreachable: String,
    },
}

/// One file, as either backing returned it.
pub(crate) struct Content {
    /// The path that was read, which is not always the path that was asked for
    /// — see [`Checkout::read`].
    pub path: String,
    pub body: Body,
    /// The type the Sandbox reported. A snapshot stores no metadata beside the
    /// bytes, so a caller that needs one falls back to the extension.
    pub media_type: Option<String>,
}

/// A file's bytes, and whether they are text.
///
/// The distinction is the file's, not the transport's: the Sandbox base64-encodes
/// what it cannot decode as UTF-8 and a snapshot hands back raw bytes, and both
/// arrive here as the same two cases.
pub(crate) enum Body {
    Text(String),
    Binary(Vec<u8>),
}

impl Body {
    pub(crate) fn into_bytes(self) -> Vec<u8> {
        match self {
            Body::Text(text) => text.into_bytes(),
            Body::Binary(bytes) => bytes,
        }
    }
}

impl<'a> Checkout<'a> {
    /// Open `task_id`'s files, live when its Sandbox answers and from the
    /// snapshot when there is no Sandbox to answer.
    ///
    /// [`no_live_sandbox`] is the whole of that distinction: a Sandbox that is
    /// up and refused the connection has current files, and answering it from a
    /// stored copy would present an older tree as the live one.
    pub(crate) async fn open(
        sources: Sources<'a>,
        task_id: &str,
    ) -> Result<Self, WorkspaceAccessError> {
        let backing = match sources.sandboxes.sandbox_client(task_id).await {
            Ok(client) => Backing::Live(client),
            Err(error) if no_live_sandbox(&error) => match sources.snapshots {
                Some(source) => Backing::Stored {
                    source,
                    unreachable: error.to_string(),
                },
                None => return Err(WorkspaceAccessError::NotRunning(error)),
            },
            Err(error) => return Err(workspace::sandbox_access(error)),
        };
        Ok(Self {
            task_id: task_id.to_string(),
            backing,
        })
    }

    /// Every path in the workspace, a directory suffixed with `/`.
    ///
    /// Honours the repositories' ignore rules whichever way it is answered: the
    /// live walk is asked to, and the mirror only ever stored the non-ignored
    /// set.
    pub(crate) async fn listing(&self) -> Result<Vec<String>, WorkspaceAccessError> {
        match &self.backing {
            Backing::Live(client) => client
                .list_files_for_task(Some(&self.task_id), ".", true, true)
                .await
                .map_err(|status| sandbox_failed(status, "list_files")),
            Backing::Stored {
                source,
                unreachable,
            } => {
                let listing = source
                    .list(&self.task_id, ".", true)
                    .await
                    .map_err(|error| self.stored_failed(error, ".", unreachable))?;
                Ok(listing
                    .entries
                    .into_iter()
                    .map(|entry| {
                        if entry.is_dir {
                            format!("{}/", entry.path)
                        } else {
                            entry.path
                        }
                    })
                    .collect())
            }
        }
    }

    /// Up to `limit` files whose name matches `query`, case-insensitively.
    ///
    /// Files only. The live search walks for files
    /// (`crates/sandbox/src/files/search.rs`) and the mirror stores nothing else, so
    /// there is no directory here to carry a trailing slash.
    pub(crate) async fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<String>, WorkspaceAccessError> {
        match &self.backing {
            Backing::Live(client) => {
                let found = client
                    .search_files_by_name_for_task(
                        Some(&self.task_id),
                        query,
                        false,
                        i32::try_from(limit).unwrap_or(i32::MAX),
                    )
                    .await
                    .map_err(|status| sandbox_failed(status, "search_files_by_name"))?;
                Ok(found.files.into_iter().map(|found| found.path).collect())
            }
            Backing::Stored {
                source,
                unreachable,
            } => {
                let found = source
                    .search(&self.task_id, query, false, limit)
                    .await
                    .map_err(|error| self.stored_failed(error, query, unreachable))?;
                Ok(found.entries.into_iter().map(|entry| entry.path).collect())
            }
        }
    }

    /// One file, resolving a path that names it from inside a checkout.
    ///
    /// A miss at the workspace root is retried against
    /// [`crate::checkout::nested_match`], because the paths an agent writes into
    /// a transcript are relative to the repository it works in and this module
    /// addresses from the root. The [`Content::path`] that comes back is the one
    /// that was read.
    pub(crate) async fn read(&self, relative_path: &str) -> Result<Content, WorkspaceAccessError> {
        let miss = match self.read_at(relative_path).await {
            Ok(content) => return Ok(content),
            Err(WorkspaceAccessError::Sandbox(SandboxError::NotFound(message))) => message,
            Err(error) => return Err(error),
        };
        match self.nested(relative_path).await {
            Some(nested) => self.read_at(&nested).await,
            // Nothing in the workspace is called this, so the miss is the
            // answer.
            None => Err(WorkspaceAccessError::Sandbox(SandboxError::NotFound(miss))),
        }
    }

    /// One file at exactly this path.
    ///
    /// Without the git overlay: no caller of this module shows a diff, so
    /// asking for one would cost a `git` invocation per opened file.
    async fn read_at(&self, path: &str) -> Result<Content, WorkspaceAccessError> {
        match &self.backing {
            Backing::Live(client) => {
                let file = client
                    .read_file_for_task(Some(&self.task_id), path, false)
                    .await
                    .map_err(|status| sandbox_failed(status, "read_file"))?;
                let body = if file.is_binary.unwrap_or(false) {
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(&file.content)
                        .map_err(|error| {
                            warn!(task_id = %self.task_id, %path, %error, "Sandbox returned content that is not base64");
                            WorkspaceAccessError::Sandbox(SandboxError::Other(format!(
                                "{path} could not be decoded"
                            )))
                        })?;
                    Body::Binary(bytes)
                } else {
                    Body::Text(file.content)
                };
                Ok(Content {
                    path: path.to_string(),
                    body,
                    media_type: file.media_type,
                })
            }
            Backing::Stored {
                source,
                unreachable,
            } => {
                let file = source
                    .read(&self.task_id, path)
                    .await
                    .map_err(|error| self.stored_failed(error, path, unreachable))?;
                let body = match String::from_utf8(file.content) {
                    Ok(text) => Body::Text(text),
                    Err(error) => Body::Binary(error.into_bytes()),
                };
                Ok(Content {
                    path: path.to_string(),
                    body,
                    media_type: None,
                })
            }
        }
    }

    /// The workspace path a repository-relative path names, when the workspace
    /// root has no such file. See [`nested_match`].
    async fn nested(&self, relative_path: &str) -> Option<String> {
        let needle = relative_path.trim_start_matches("./").trim_end_matches('/');
        if needle.is_empty() {
            return None;
        }
        let listing = self
            .listing()
            .await
            .inspect_err(|error| {
                warn!(
                    ?error,
                    "Could not list the workspace to resolve a repository-relative path"
                );
            })
            .ok()?;
        nested_match(&listing, needle).map(str::to_string)
    }

    /// What a caller is told when the snapshot could not answer.
    ///
    /// `NoSnapshot` reads back as the Sandbox failure that sent the read here:
    /// a Task with nothing running and nothing stored is not running, and
    /// naming the store would tell the person about a mechanism instead of
    /// about their Task.
    fn stored_failed(
        &self,
        error: SnapshotError,
        what: &str,
        unreachable: &str,
    ) -> WorkspaceAccessError {
        match error {
            SnapshotError::NoSnapshot => {
                WorkspaceAccessError::NotRunning(SandboxError::NotReady(unreachable.to_string()))
            }
            SnapshotError::NotFound => {
                WorkspaceAccessError::Sandbox(SandboxError::NotFound(what.to_string()))
            }
            SnapshotError::Unavailable(message) => {
                warn!(task_id = %self.task_id, %message, "Could not read the stored workspace");
                WorkspaceAccessError::Sandbox(SandboxError::Other(message))
            }
        }
    }
}

fn sandbox_failed(status: tonic::Status, operation: &str) -> WorkspaceAccessError {
    WorkspaceAccessError::Sandbox(classify_grpc_error(status, operation))
}

/// The one entry of `listing` that `needle` names from inside a checkout.
///
/// A Moatless workspace is a *set* of checkouts under one root, and the agent
/// works inside one of them — so the paths it writes into a transcript,
/// `backend/migrations/0125.sql`, are relative to its repository and not to the
/// root this module presents. The client has no way to know that: the `cwd` it
/// resolves a mention against is the workspace root, so what arrives is the path
/// the agent wrote, one directory short of a file.
///
/// Matching is on whole segments against the listing the tree itself shows, so
/// this can only ever name a file the person could already see, and it costs a
/// walk only on the miss. The shallowest match wins — the checkout nearest the
/// root — and equal-length ones break by name, so the same mention always opens
/// the same file rather than depending on walk order.
pub(crate) fn nested_match<'l>(listing: &'l [String], needle: &str) -> Option<&'l str> {
    let suffix = format!("/{needle}");
    listing
        .iter()
        // Directories carry a trailing slash and are not files to open.
        .filter(|path| !path.ends_with('/') && path.ends_with(&suffix))
        .map(String::as_str)
        .min_by_key(|path| (path.len(), *path))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The mention an agent writes is relative to the checkout it works in, so
    /// the path that arrives is one directory short of the file.
    #[test]
    fn a_path_relative_to_a_checkout_names_the_file_inside_it() {
        let listing = [
            "moatless/".to_string(),
            "moatless/backend/".to_string(),
            "moatless/backend/migrations/0126_tasks_closed_at.sql".to_string(),
            "t3code/package.json".to_string(),
        ];
        assert_eq!(
            nested_match(&listing, "backend/migrations/0126_tasks_closed_at.sql"),
            Some("moatless/backend/migrations/0126_tasks_closed_at.sql"),
        );
    }

    /// Segment boundaries, not substrings: `grations/0125.sql` names nothing,
    /// and neither does a directory that happens to end the same way.
    #[test]
    fn a_match_is_on_whole_segments_and_only_on_files() {
        let listing = [
            "moatless/backend/migrations/0125.sql".to_string(),
            "moatless/backend/migrations/".to_string(),
        ];
        assert_eq!(nested_match(&listing, "grations/0125.sql"), None);
        assert_eq!(nested_match(&listing, "backend/migrations"), None);
        assert_eq!(
            nested_match(&listing, "migrations/0125.sql"),
            Some("moatless/backend/migrations/0125.sql"),
        );
    }

    /// Two checkouts of the same shape are the ordinary case in a multi-repo
    /// workspace, and a mention that fits both has to keep opening the same one
    /// however the walk happened to be ordered.
    #[test]
    fn the_shallowest_match_wins_and_ties_break_by_name() {
        let listing = [
            "zzz/src/main.rs".to_string(),
            "vendored/deep/tree/src/main.rs".to_string(),
            "api/src/main.rs".to_string(),
        ];
        assert_eq!(
            nested_match(&listing, "src/main.rs"),
            Some("api/src/main.rs")
        );
    }

    /// The workspace root's own files are found by the read itself, so this is
    /// only ever asked about paths that are not there.
    #[test]
    fn a_path_that_is_nowhere_in_the_workspace_resolves_to_nothing() {
        let listing = ["moatless/backend/migrations/0125.sql".to_string()];
        assert_eq!(nested_match(&listing, "nowhere/at/all.sql"), None);
    }
}
