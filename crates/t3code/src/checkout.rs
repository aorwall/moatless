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

    /// One file, resolving a path an agent wrote rather than one this module
    /// issued.
    ///
    /// A path the workspace root holds is read as it stands, and anything else
    /// goes to [`Self::resolve`]. The [`Content::path`] that comes back is the
    /// one that was read.
    pub(crate) async fn read(&self, relative_path: &str) -> Result<Content, WorkspaceAccessError> {
        // Reading an absolute path is refused before either backing looks at
        // the file, so there is no miss to learn anything from.
        let miss = if relative_path.starts_with('/') {
            relative_path.to_string()
        } else {
            match self.read_at(relative_path).await {
                Ok(content) => return Ok(content),
                Err(WorkspaceAccessError::Sandbox(SandboxError::NotFound(message))) => message,
                Err(error) => return Err(error),
            }
        };
        match self.resolve(relative_path).await {
            Some(path) => self.read_at(&path).await,
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

    /// The workspace path a mention names, when this module's own addressing
    /// does not reach the file.
    ///
    /// A mention arrives either short of the workspace root or past it, and the
    /// string does not say which: the asset route spends a Sandbox path's
    /// leading `/` on the URL it is a segment of, and a proxy that merges
    /// duplicate slashes would eat it anyway. So the root is stripped and both
    /// readings are tried, [`nested_match`] first because a repository-relative
    /// mention is the one an agent writes most.
    async fn resolve(&self, mention: &str) -> Option<String> {
        let needle = mention
            .trim_start_matches('/')
            .trim_start_matches("./")
            .trim_end_matches('/');
        if needle.is_empty() {
            return None;
        }
        let listing = self
            .listing()
            .await
            .inspect_err(|error| {
                warn!(?error, "Could not list the workspace to resolve a mention");
            })
            .ok()?;
        nested_match(&listing, needle)
            .or_else(|| rooted_match(&listing, &format!("/{needle}")))
            .map(str::to_string)
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

/// The one entry of `listing` that an absolute `needle` names.
///
/// An agent works inside the Sandbox and writes the path it used there, so a
/// transcript carries `/opt/moatless/workspace/moatless/README.md` as readily as
/// it carries a repository-relative path. Both name a file the tree already
/// shows, and neither is addressable as it stands: this module addresses from
/// the workspace root, and the Sandbox refuses a leading `/` before it opens
/// anything (`crates/sandbox/src/files/path.rs`).
///
/// Where that root sits on the Sandbox's disk is the Sandbox's own business and
/// no field this crate is given, so the match is on the listing instead: the
/// longest entry the absolute path ends on, at a segment boundary. Longest
/// rather than shallowest, deliberately — every shorter match is a different
/// file that happens to share a tail, and the longest is the one the person
/// named. A path that ends on nothing in the listing stays unresolved, which is
/// what keeps `/etc/passwd` out of a workspace read.
pub(crate) fn rooted_match<'l>(listing: &'l [String], needle: &str) -> Option<&'l str> {
    listing
        .iter()
        // Directories carry a trailing slash and are not files to open.
        .filter(|path| !path.ends_with('/'))
        .map(String::as_str)
        .filter(|path| {
            needle
                .strip_suffix(*path)
                .is_some_and(|root| root.ends_with('/'))
        })
        .max_by_key(|path| path.len())
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use moatless_ports::sandbox::{
        MockWorkspaceSnapshotSource, SnapshotEntry, SnapshotFile as StoredFile, SnapshotListing,
    };

    use super::*;

    /// A connector that never has a Sandbox, which is what sends every read in
    /// these tests to the stored backing.
    struct NoSandbox;

    #[async_trait::async_trait]
    impl SandboxConnector for NoSandbox {
        async fn sandbox_client(&self, _task_id: &str) -> Result<SandboxClient, SandboxError> {
            Err(SandboxError::NotReady("sandbox is stopped".into()))
        }
    }

    fn listing(paths: &[(&str, bool)]) -> SnapshotListing {
        SnapshotListing {
            entries: paths
                .iter()
                .map(|(path, is_dir)| SnapshotEntry {
                    path: (*path).to_string(),
                    is_dir: *is_dir,
                    size: 4,
                })
                .collect(),
            synced_at: Utc::now(),
        }
    }

    async fn stored(source: MockWorkspaceSnapshotSource) -> Checkout<'static> {
        // Leaked so the checkout can borrow it for the length of the test; a
        // test process is the whole lifetime either way.
        let sources = Sources {
            sandboxes: Box::leak(Box::new(NoSandbox)),
            snapshots: Some(Box::leak(Box::new(source))),
        };
        Checkout::open(sources, "task-1")
            .await
            .expect("a stopped Task falls back to the store")
    }

    /// The client renders a tree, so a directory has to arrive with the
    /// trailing slash that tells it from a file.
    #[tokio::test]
    async fn a_stored_listing_marks_its_directories_with_a_trailing_slash() {
        let mut source = MockWorkspaceSnapshotSource::new();
        source
            .expect_list()
            .withf(|task_id, path, recursive| task_id == "task-1" && path == "." && *recursive)
            .returning(|_, _, _| Ok(listing(&[("repo", true), ("repo/main.rs", false)])));

        assert_eq!(
            stored(source).await.listing().await.unwrap(),
            vec!["repo/".to_string(), "repo/main.rs".to_string()]
        );
    }

    /// A stopped Task with nothing stored is still just a Task that is not
    /// running — naming the store would tell the person about a mechanism.
    #[tokio::test]
    async fn a_task_with_nothing_stored_reads_as_not_running() {
        let mut source = MockWorkspaceSnapshotSource::new();
        source
            .expect_list()
            .returning(|_, _, _| Err(SnapshotError::NoSnapshot));

        let Err(WorkspaceAccessError::NotRunning(SandboxError::NotReady(reason))) =
            stored(source).await.listing().await
        else {
            panic!("a Task with nothing stored is a Task that is not running");
        };
        assert!(
            reason.contains("sandbox is stopped"),
            "the reason stays the Sandbox's, not the store's: {reason}"
        );
    }

    /// The store hands back raw bytes and says nothing about their type, so
    /// this is where text and binary are told apart.
    #[tokio::test]
    async fn stored_bytes_that_are_not_utf8_read_back_as_binary() {
        let mut source = MockWorkspaceSnapshotSource::new();
        source.expect_read().returning(|_, path| {
            let content = if path == "logo.png" {
                vec![0xff, 0xd8, 0xff]
            } else {
                b"hello".to_vec()
            };
            Ok(StoredFile {
                content,
                synced_at: Utc::now(),
            })
        });
        let checkout = stored(source).await;

        let text = checkout.read("notes.md").await.unwrap();
        assert!(matches!(text.body, Body::Text(ref body) if body == "hello"));
        assert_eq!(text.media_type, None, "a snapshot stores no media type");

        let binary = checkout.read("logo.png").await.unwrap();
        assert!(matches!(binary.body, Body::Binary(ref bytes) if bytes == &[0xff, 0xd8, 0xff]));
    }

    /// The mention an agent writes is relative to its checkout, and the stored
    /// backing has to resolve it the same way the live one does: on a miss,
    /// against the listing.
    #[tokio::test]
    async fn a_stored_read_resolves_a_path_relative_to_a_checkout() {
        let mut source = MockWorkspaceSnapshotSource::new();
        source.expect_read().returning(|_, path| {
            if path == "repo/src/main.rs" {
                Ok(StoredFile {
                    content: b"fn main() {}".to_vec(),
                    synced_at: Utc::now(),
                })
            } else {
                Err(SnapshotError::NotFound)
            }
        });
        source
            .expect_list()
            .returning(|_, _, _| Ok(listing(&[("repo", true), ("repo/src/main.rs", false)])));

        let found = stored(source).await.read("src/main.rs").await.unwrap();
        assert_eq!(found.path, "repo/src/main.rs");
    }

    /// An outage is not "no files": it has to reach the caller as a failure,
    /// not as a Task that is merely stopped.
    #[tokio::test]
    async fn an_unreadable_store_is_a_failure_rather_than_an_empty_task() {
        let mut source = MockWorkspaceSnapshotSource::new();
        source
            .expect_search()
            .returning(|_, _, _, _| Err(SnapshotError::Unavailable("s3 timed out".into())));

        assert!(matches!(
            stored(source).await.search("main", 10).await,
            Err(WorkspaceAccessError::Sandbox(SandboxError::Other(_)))
        ));
    }

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

    /// The other shape a mention arrives in: the path the agent used inside its
    /// Sandbox, which no caller here can subtract the workspace root from.
    #[test]
    fn an_absolute_path_names_the_file_it_ends_on() {
        let listing = [
            "moatless/".to_string(),
            "moatless/README.md".to_string(),
            "moatless/crates/t3code/src/files.rs".to_string(),
        ];
        assert_eq!(
            rooted_match(
                &listing,
                "/opt/moatless/workspace/moatless/crates/t3code/src/files.rs"
            ),
            Some("moatless/crates/t3code/src/files.rs"),
        );
        assert_eq!(
            rooted_match(&listing, "/opt/moatless/workspace/moatless/README.md"),
            Some("moatless/README.md"),
        );
    }

    /// Every shorter tail is a different file that happens to end the same way,
    /// so the longest one is the only match that is the file the person named.
    #[test]
    fn the_longest_match_wins_and_a_partial_segment_is_not_one() {
        let listing = [
            "README.md".to_string(),
            "moatless/README.md".to_string(),
            "docs/moatless/README.md".to_string(),
        ];
        assert_eq!(
            rooted_match(&listing, "/opt/moatless/workspace/docs/moatless/README.md"),
            Some("docs/moatless/README.md"),
        );
        assert_eq!(
            rooted_match(&["adme.md".to_string()], "/opt/workspace/README.md"),
            None,
            "a tail that stops inside a segment names nothing",
        );
    }

    /// A path outside the workspace ends on nothing the tree shows, and this is
    /// what stops the read from reaching it.
    #[test]
    fn an_absolute_path_outside_the_workspace_resolves_to_nothing() {
        let listing = [
            "moatless/README.md".to_string(),
            "moatless/passwd".to_string(),
        ];
        assert_eq!(rooted_match(&listing, "/etc/passwd"), None);
    }

    /// The whole point of resolving before the read: neither backing accepts a
    /// leading `/`, so an unresolved absolute path costs a round trip that can
    /// only fail.
    #[tokio::test]
    async fn an_absolute_read_opens_the_file_it_ends_on() {
        let mut source = MockWorkspaceSnapshotSource::new();
        source.expect_read().returning(|_, path| {
            if path == "repo/src/main.rs" {
                Ok(StoredFile {
                    content: b"fn main() {}".to_vec(),
                    synced_at: Utc::now(),
                })
            } else {
                Err(SnapshotError::NotFound)
            }
        });
        source
            .expect_list()
            .returning(|_, _, _| Ok(listing(&[("repo", true), ("repo/src/main.rs", false)])));

        let checkout = stored(source).await;
        for mention in [
            "/opt/moatless/workspace/repo/src/main.rs",
            // The asset route's form: the root spent on the URL it is a
            // segment of.
            "opt/moatless/workspace/repo/src/main.rs",
        ] {
            let found = checkout.read(mention).await.unwrap();
            assert_eq!(found.path, "repo/src/main.rs", "{mention}");
        }
    }

    /// The person asked for a file, so they are told it is not here — not that
    /// the request itself was malformed, which sends them nowhere.
    #[tokio::test]
    async fn an_absolute_read_that_matches_nothing_is_a_miss_on_the_path_asked_for() {
        let mut source = MockWorkspaceSnapshotSource::new();
        source
            .expect_list()
            .returning(|_, _, _| Ok(listing(&[("repo/src/main.rs", false)])));

        let Err(WorkspaceAccessError::Sandbox(SandboxError::NotFound(missed))) =
            stored(source).await.read("/etc/passwd").await
        else {
            panic!("a path the workspace does not hold is a miss");
        };
        assert_eq!(missed, "/etc/passwd");
    }
}
