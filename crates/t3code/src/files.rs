//! The workspace-files half of the T3 compatibility module.
//!
//! Moatless has no directory to point at: the nominal `cwd` this module answers
//! is the thread's `worktreePath`, from which
//! [`workspace::addressed_conversation`] resolves the conversation.
//!
//! **A read is addressed from the workspace root, and a mention is not** — see
//! [`crate::checkout::nested_match`] and [`crate::checkout::rooted_match`].

use serde_json::{Value, json};
use tracing::warn;

use super::checkout::{Body, Checkout, Sources};
use super::workspace::{self, WorkspaceAccessError};
use crate::reads::T3Reads;
use moatless_ports::sandbox::SandboxError;
use moatless_types::access_control::ScopeContext;
use sandbox_proto::client::{SandboxConnector, classify_grpc_error};

/// The most entries one listing carries, matching the reference server's own
/// index cap.
///
/// A Sandbox walk has no bound of its own — a populated `target/` or `.venv`
/// could put megabytes of paths into one frame.
const MAX_ENTRIES: usize = 25_000;

/// The most file content one read carries, matching the reference server's cap.
///
/// A file past it comes back with `truncated` set; `byteLength` still reports
/// the whole file's size.
const MAX_FILE_BYTES: usize = 1024 * 1024;

/// One conversation's whole file tree, in the shape the contract's
/// `projects.listEntries` returns.
pub async fn list_entries(
    reads: &T3Reads,
    sources: Sources<'_>,
    viewer: &ScopeContext,
    cwd: &str,
) -> Result<Value, WorkspaceAccessError> {
    let conversations = reads.conversations();
    let task_id = workspace::addressed_conversation(conversations, viewer, cwd).await?;
    // Recursive, because the contract has no second call: the client is given
    // the whole tree once and expands it locally. This is the same shape
    // Moatless's own file view fetches.
    let files = Checkout::open(sources, &task_id).await?.listing().await?;

    let truncated = files.len() > MAX_ENTRIES;
    if truncated {
        warn!(
            task_id = %task_id,
            walked = files.len(),
            cap = MAX_ENTRIES,
            "Sandbox file listing exceeded the T3 UI entry cap; reporting it truncated"
        );
    }
    let entries: Vec<Value> = share_cap(&files, MAX_ENTRIES)
        .into_iter()
        .filter_map(entry)
        .collect();
    Ok(json!({ "entries": entries, "truncated": truncated }))
}

/// The listing cut to `cap`, with each top-level entry holding its own share.
///
/// Dot entries are served in a second round; smallest group first, and each
/// group keeps its shallowest paths so ancestors stay kept.
fn share_cap(files: &[String], cap: usize) -> Vec<&str> {
    let mut paths: Vec<&str> = files.iter().map(String::as_str).collect();
    // Sorted here rather than relied on: the live walk sorts, the snapshot
    // backing does not.
    paths.sort_unstable();
    if paths.len() <= cap {
        return paths;
    }

    let mut groups: Vec<&[&str]> = Vec::new();
    let mut start = 0;
    while start < paths.len() {
        let name = top_level(paths[start]);
        let end = paths[start..]
            .iter()
            .position(|path| top_level(path) != name)
            .map_or(paths.len(), |offset| start + offset);
        groups.push(&paths[start..end]);
        start = end;
    }

    let (dotted, plain): (Vec<usize>, Vec<usize>) =
        (0..groups.len()).partition(|&group| groups[group][0].starts_with('.'));
    let mut taken = vec![0usize; groups.len()];
    let mut budget = cap;
    for mut round in [plain, dotted] {
        round.sort_unstable_by_key(|&group| groups[group].len());
        let mut unserved = round.len();
        for group in round {
            let share = budget / unserved;
            taken[group] = groups[group].len().min(share);
            budget -= taken[group];
            unserved -= 1;
        }
    }

    let mut kept: Vec<&str> = groups
        .iter()
        .zip(&taken)
        .flat_map(|(group, &share)| shallowest(group, share))
        .collect();
    kept.sort_unstable();
    kept
}

/// The `share` shallowest paths of `group`, ties broken by name.
fn shallowest<'a>(group: &[&'a str], share: usize) -> Vec<&'a str> {
    // The dot round leaves a group at zero whenever the checkouts spent the
    // cap, so this is the common path and not a corner.
    if share == 0 {
        return Vec::new();
    }
    if share == group.len() {
        return group.to_vec();
    }
    let mut ordered = group.to_vec();
    ordered.sort_unstable_by_key(|path| (depth(path), *path));
    ordered.truncate(share);
    ordered
}

/// How many directories `path` hangs under. A trailing slash marks a directory
/// and does not count as one of them.
fn depth(path: &str) -> usize {
    path.strip_suffix('/').unwrap_or(path).matches('/').count()
}

/// The first segment of `path`, which is the entry the workspace root holds it
/// under. A root-level file is its own segment, and so its own group.
fn top_level(path: &str) -> &str {
    path.split('/').next().unwrap_or(path)
}

/// One entry of a Sandbox listing, in the contract's shape.
///
/// A trailing slash marks a directory, its absence a file; a bare slash is
/// dropped since the contract refuses a blank path.
fn entry(path: &str) -> Option<Value> {
    let (path, kind) = match path.strip_suffix('/') {
        Some(directory) => (directory, "directory"),
        None => (path, "file"),
    };
    (!path.is_empty()).then(|| json!({ "path": path, "kind": kind }))
}

/// Which files here are called something like this (`projects.searchEntries`).
///
/// An empty query is a bounded browse, not a search, answered from the same
/// listing [`list_entries`] reads.
pub async fn search_entries(
    reads: &T3Reads,
    sources: Sources<'_>,
    viewer: &ScopeContext,
    cwd: &str,
    query: &str,
    limit: usize,
    kind: Option<&str>,
) -> Result<Value, WorkspaceAccessError> {
    let conversations = reads.conversations();
    let task_id = workspace::addressed_conversation(conversations, viewer, cwd).await?;
    let checkout = Checkout::open(sources, &task_id).await?;

    let (mut entries, truncated) = if query.is_empty() {
        let files = checkout.listing().await?;
        let truncated = files.len() > limit;
        let entries: Vec<Value> = share_cap(&files, limit)
            .into_iter()
            .filter_map(entry)
            .collect();
        (entries, truncated)
    } else {
        let found = checkout.search(query, limit).await?;
        let truncated = found.len() >= limit;
        // Always files: neither backing has a directory to offer here — the
        // Sandbox's walker skips anything that is not a file
        // (`crates/sandbox/src/files/search.rs:69`) and the mirror stores only files.
        let entries: Vec<Value> = found
            .iter()
            .filter(|path| !path.is_empty())
            .map(|path| json!({ "path": path, "kind": "file" }))
            .collect();
        (entries, truncated)
    };

    if let Some(kind) = kind {
        entries.retain(|entry| entry["kind"] == kind);
    }
    Ok(json!({ "entries": entries, "truncated": truncated }))
}

/// A file written into the conversation's worktree (`projects.writeFile`).
///
/// Gated by [`workspace::controlled_conversation`], not `addressed_`; never
/// resolved like [`read_file`], or mention resolution would overwrite instead
/// of creating.
pub async fn write_file(
    reads: &T3Reads,
    sandboxes: &dyn SandboxConnector,
    viewer: &ScopeContext,
    cwd: &str,
    relative_path: &str,
    contents: &str,
) -> Result<Value, WorkspaceAccessError> {
    let conversations = reads.conversations();
    let task_id = workspace::controlled_conversation(conversations, viewer, cwd).await?;
    sandboxes
        .sandbox_client(&task_id)
        .await
        .map_err(WorkspaceAccessError::NotRunning)?
        .write_file_for_task(Some(&task_id), relative_path, contents)
        .await
        .map_err(|status| {
            WorkspaceAccessError::Sandbox(classify_grpc_error(status, "write_file"))
        })?;

    // The path and nothing else: `ProjectWriteFileResult` carries one field
    // (`packages/contracts/src/project.ts:271`).
    Ok(json!({ "relativePath": relative_path }))
}

/// One file's contents (`projects.readFile`).
///
/// The returned path is the one actually read, not necessarily the one asked
/// for ([`crate::checkout::nested_match`], [`crate::checkout::rooted_match`]).
pub async fn read_file(
    reads: &T3Reads,
    sources: Sources<'_>,
    viewer: &ScopeContext,
    cwd: &str,
    relative_path: &str,
) -> Result<Value, WorkspaceAccessError> {
    let conversations = reads.conversations();
    let task_id = workspace::addressed_conversation(conversations, viewer, cwd).await?;
    let file = Checkout::open(sources, &task_id)
        .await?
        .read(relative_path)
        .await?;

    let Body::Text(contents) = file.body else {
        // Returning the base64 as `contents` would put the encoding in the
        // editor as if it were the file, so this is refused in the contract's
        // own terms instead.
        return Err(WorkspaceAccessError::Sandbox(SandboxError::BadRequest(
            format!("{} is a binary file", file.path),
        )));
    };

    let byte_length = contents.len();
    let contents = truncate_on_char_boundary(&contents, MAX_FILE_BYTES);
    Ok(json!({
        "relativePath": file.path,
        "contents": contents,
        // The file's size, not the excerpt's — the client shows it as the file's.
        "byteLength": byte_length,
        "truncated": contents.len() < byte_length
    }))
}

/// The longest prefix of `contents` within `max_bytes` that is still text.
///
/// A byte-offset cut can land inside a UTF-8 character, so it moves back to the
/// nearest boundary.
fn truncate_on_char_boundary(contents: &str, max_bytes: usize) -> &str {
    if contents.len() <= max_bytes {
        return contents;
    }
    let mut end = max_bytes;
    while end > 0 && !contents.is_char_boundary(end) {
        end -= 1;
    }
    &contents[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trailing_slash_is_the_whole_of_the_file_directory_distinction() {
        assert_eq!(
            entry("src/"),
            Some(json!({ "path": "src", "kind": "directory" })),
        );
        assert_eq!(
            entry("src/main.rs"),
            Some(json!({ "path": "src/main.rs", "kind": "file" })),
        );
        assert_eq!(entry("/"), None, "a bare slash carries no path");
        assert_eq!(entry(""), None);
    }

    /// A workspace root holding a dependency cache beside its checkouts, which
    /// is what a Sandbox looks like after an agent has run an installer in it.
    fn workspace(cache_packages: usize, repo_files: usize) -> Vec<String> {
        let mut files = vec![
            ".cache/".to_string(),
            "repo/".to_string(),
            "repo/src/".to_string(),
        ];
        for n in 0..cache_packages {
            files.push(format!(".cache/package-{n:05}/"));
            files.push(format!(".cache/package-{n:05}/index.js"));
        }
        files.extend((0..repo_files).map(|n| format!("repo/src/file-{n:05}.rs")));
        files
    }

    #[test]
    fn a_cache_bigger_than_the_cap_does_not_hide_the_checkouts() {
        let files = workspace(30_000, 400);
        let kept = share_cap(&files, 1_000);

        assert_eq!(kept.len(), 1_000);
        assert_eq!(
            kept.iter().filter(|path| path.starts_with("repo")).count(),
            402,
            "the repository is under its share, so all of it is kept"
        );
        assert!(
            kept.contains(&"repo/"),
            "the client has a row to hang it on"
        );
    }

    #[test]
    fn a_share_the_small_entries_do_not_use_goes_to_the_cut_ones() {
        // Twelve paths outside the cache, so the cache takes the other 988
        // rather than the 500 an equal split would have given it.
        let files = workspace(30_000, 10);
        let kept = share_cap(&files, 1_000);
        assert_eq!(kept.len(), 1_000);
        assert_eq!(
            kept.iter()
                .filter(|path| path.starts_with(".cache"))
                .count(),
            988,
        );
    }

    #[test]
    fn a_dot_entry_takes_only_what_the_checkouts_leave() {
        // The repository alone is over the cap, so the cache gets nothing.
        let files = workspace(30_000, 2_000);
        let kept = share_cap(&files, 1_000);
        assert_eq!(kept.len(), 1_000);
        assert_eq!(
            kept.iter()
                .filter(|path| path.starts_with(".cache"))
                .count(),
            0,
        );
    }

    #[test]
    fn a_root_of_only_dot_entries_still_spends_the_cap() {
        // A sandbox with no repo to clone, so the first round is empty.
        let mut files = vec![".cache/".to_string(), ".moatless/".to_string()];
        files.extend((0..3_000).map(|n| format!(".cache/package-{n:05}")));
        assert_eq!(share_cap(&files, 1_000).len(), 1_000);
    }

    #[test]
    fn the_two_rounds_compose_over_two_checkouts() {
        let mut files = vec![
            "alpha/".to_string(),
            "beta/".to_string(),
            ".cache/".to_string(),
        ];
        files.extend((0..100).map(|n| format!("alpha/file-{n:05}.rs")));
        files.extend((0..5_000).map(|n| format!("beta/file-{n:05}.rs")));
        files.extend((0..10_000).map(|n| format!(".cache/package-{n:05}")));

        let kept = share_cap(&files, 1_000);
        let count = |prefix: &str| kept.iter().filter(|path| path.starts_with(prefix)).count();
        assert_eq!(count("alpha"), 101, "under its share, so kept whole");
        assert_eq!(count("beta"), 899, "its share, plus what alpha left");
        assert_eq!(count(".cache"), 0, "the checkouts spent the cap");
    }

    #[test]
    fn a_cut_group_keeps_the_top_of_its_tree() {
        // The share stops inside the third level, so ordering by name alone
        // would spend it all inside `repo/a/` and drop the two siblings.
        let mut files = vec![
            "repo/".to_string(),
            "repo/a/".to_string(),
            "repo/b/".to_string(),
            "repo/c/".to_string(),
        ];
        for directory in ["a", "b", "c"] {
            files.extend((0..100).map(|n| format!("repo/{directory}/file-{n:05}.rs")));
        }

        let kept = share_cap(&files, 20);
        assert_eq!(kept.len(), 20);
        assert!(kept.contains(&"repo/b/"), "the second directory is listed");
        assert!(kept.contains(&"repo/c/"), "the third directory is listed");
    }

    #[test]
    fn every_kept_path_keeps_the_directories_it_hangs_under() {
        let files = workspace(30_000, 400);
        let kept = share_cap(&files, 1_000);
        let directories: std::collections::HashSet<&str> = kept
            .iter()
            .filter_map(|path| path.strip_suffix('/'))
            .collect();
        for path in &kept {
            let path = path.strip_suffix('/').unwrap_or(path);
            if let Some((parent, _)) = path.rsplit_once('/') {
                assert!(directories.contains(parent), "{path} hangs under {parent}");
            }
        }
    }

    #[test]
    fn a_listing_inside_the_cap_is_kept_whole() {
        let files = workspace(10, 10);
        assert_eq!(share_cap(&files, 1_000).len(), files.len());
    }

    #[test]
    fn a_cut_never_lands_inside_a_character() {
        // Three bytes each, so every cap that is not a multiple of three falls
        // inside one and has to move back.
        let contents = "€€€";
        for cap in 0..=contents.len() {
            let cut = truncate_on_char_boundary(contents, cap);
            assert!(cut.len() <= cap);
            assert_eq!(cut.len() % 3, 0, "{cap} cut inside a character");
        }
        assert_eq!(truncate_on_char_boundary(contents, 99), contents);
    }

    /// One Sandbox, reached at a URL instead of by service name — the
    /// k8s/Docker adapters' own addressing can't be reached from a test.
    #[cfg(feature = "live-sandbox-tests")]
    struct SandboxAt(String);

    #[cfg(feature = "live-sandbox-tests")]
    #[async_trait::async_trait]
    impl SandboxConnector for SandboxAt {
        async fn sandbox_client(
            &self,
            _task_id: &str,
        ) -> Result<sandbox_proto::client::SandboxClient, SandboxError> {
            sandbox_proto::client::SandboxClient::connect(&self.0)
                .map_err(|error| SandboxError::GrpcClient(error.to_string()))
        }
    }

    /// A conversation with a running Sandbox, and nothing else filled in: the
    /// only fields this module reads are the id and the Sandbox's intent.
    #[cfg(feature = "live-sandbox-tests")]
    fn running(task_id: &str) -> crate::dao::Conversation {
        crate::dao::Conversation {
            task_id: task_id.to_string(),
            workspace_id: Some("ws".into()),
            name: None,
            branch: None,
            agent_profile: moatless_types::agent::AgentProfile::ClaudeCode.into(),
            created_at: "2026-01-01T00:00:00Z".into(),
            created_at_db: None,
            updated_at: None,
            closed_at: None,
            turn_number: None,
            turn_status: None,
            turn_started_at: None,
            turn_ended_at: None,
            sandbox_desired_state: Some(moatless_types::sandbox::SandboxDesiredState::Running),
            sandbox_stop_reason_raw: None,
            sandbox_started_at: None,
            sandbox_updated_at: None,
            latest_user_message_at: None,
            latest_queued_message_at: None,
            assistant_model: None,
            requested_model: None,
            pending_question: false,
            pending_plan: false,
            settled_override: None,
            settled_at: None,
            snoozed_until: None,
            snoozed_at: None,
        }
    }

    /// The whole projection, against a real Sandbox — ignored; point
    /// `T3_UI_RPC_TEST_SANDBOX_GRPC` at one (e.g. `http://127.0.0.1:50052`) and
    /// run:
    /// ```text
    /// T3_UI_RPC_TEST_SANDBOX_GRPC=http://127.0.0.1:50052 \
    ///   cargo test -p moatless-t3code --features live-sandbox-tests files -- --ignored --nocapture
    /// ```
    #[cfg(feature = "live-sandbox-tests")]
    #[tokio::test]
    #[ignore = "needs a Sandbox at T3_UI_RPC_TEST_SANDBOX_GRPC"]
    async fn a_real_sandbox_listing_becomes_contract_entries() {
        let url = std::env::var("T3_UI_RPC_TEST_SANDBOX_GRPC")
            .expect("point T3_UI_RPC_TEST_SANDBOX_GRPC at a Sandbox's gRPC endpoint");
        let mut conversations = crate::dao::MockConversationDao::new();
        conversations
            .expect_list_followed_by_ids()
            .returning(|_, ids| Ok(vec![running(&ids[0])]));
        let viewer = ScopeContext {
            user_id: "viewer".into(),
            is_admin: false,
        };
        let cwd = "/moatless/tasks/probe";
        let connector = SandboxAt(url.clone());
        let sources = Sources {
            sandboxes: &connector,
            snapshots: None,
        };

        let listing = list_entries(&conversations, sources, &viewer, cwd)
            .await
            .expect("the sandbox answered");
        let entries = listing["entries"].as_array().expect("an entries array");
        assert!(!entries.is_empty(), "a sandbox workspace is not empty");
        println!(
            "{} entries, truncated={}",
            entries.len(),
            listing["truncated"]
        );

        let mut directories: std::collections::HashSet<&str> = std::collections::HashSet::new();
        let mut first_file: Option<&str> = None;
        for entry in entries {
            let path = entry["path"].as_str().expect("a path");
            assert_eq!(path.trim(), path, "the contract trims and refuses blanks");
            assert!(!path.is_empty() && !path.starts_with('/') && !path.ends_with('/'));
            match entry["kind"].as_str() {
                Some("directory") => {
                    directories.insert(path);
                }
                Some("file") => first_file = first_file.or(Some(path)),
                other => panic!("{other:?} is not a kind the contract has"),
            }
        }
        for entry in entries {
            let path = entry["path"].as_str().expect("a path");
            if let Some((parent, _)) = path.rsplit_once('/') {
                assert!(
                    directories.contains(parent),
                    "{path} hangs under {parent}, which the client has no row for"
                );
            }
        }

        let path = first_file.expect("a workspace with files in it");
        let file = read_file(&conversations, sources, &viewer, cwd, path)
            .await
            .expect("the sandbox answered");
        assert_eq!(file["relativePath"], path);
        assert!(file["contents"].is_string());
        assert!(file["byteLength"].as_u64().is_some());
        println!("read {path}: {} bytes", file["byteLength"]);
    }

    /// The mention case: a path relative to a checkout, with no file at that
    /// path in the workspace root, opens the file inside the checkout.
    ///
    /// Ignored for the same reason as the test above, run the same way.
    #[cfg(feature = "live-sandbox-tests")]
    #[tokio::test]
    #[ignore = "needs a Sandbox at T3_UI_RPC_TEST_SANDBOX_GRPC"]
    async fn a_real_sandbox_resolves_a_path_relative_to_one_of_its_checkouts() {
        let url = std::env::var("T3_UI_RPC_TEST_SANDBOX_GRPC")
            .expect("point T3_UI_RPC_TEST_SANDBOX_GRPC at a Sandbox's gRPC endpoint");
        let mut conversations = crate::dao::MockConversationDao::new();
        conversations
            .expect_list_followed_by_ids()
            .returning(|_, ids| Ok(vec![running(&ids[0])]));
        let viewer = ScopeContext {
            user_id: "viewer".into(),
            is_admin: false,
        };
        let cwd = "/moatless/tasks/probe";
        let connector = SandboxAt(url.clone());
        let sources = Sources {
            sandboxes: &connector,
            snapshots: None,
        };

        let listing = list_entries(&conversations, sources, &viewer, cwd)
            .await
            .expect("the sandbox answered");
        let entries = listing["entries"].as_array().expect("an entries array");
        let files: Vec<String> = entries
            .iter()
            .filter(|entry| entry["kind"] == "file")
            .filter_map(|entry| Some(entry["path"].as_str()?.to_string()))
            .collect();
        // A file inside a checkout, named the way an agent working in that
        // checkout would name it: without the checkout's own directory. It has
        // to be one the root itself has no file at, and one no other checkout
        // answers to first — otherwise the mention is genuinely ambiguous and
        // the resolution rule, not the sandbox, is what is under test.
        let (path, nested) = files
            .iter()
            .filter_map(|path| Some((path.as_str(), path.split_once('/')?.1)))
            .find(|(path, nested)| {
                !files.iter().any(|entry| entry == nested)
                    && crate::checkout::nested_match(&files, nested) == Some(*path)
            })
            .expect("a workspace with a file inside a checkout");

        println!("asking for {nested}, which is really {path}");
        let file = read_file(&conversations, sources, &viewer, cwd, nested)
            .await
            .expect("the sandbox answered");
        assert_eq!(
            file["relativePath"], path,
            "the read answers with the path it read, not the one it was asked for"
        );
        assert!(file["contents"].is_string());
    }
}
