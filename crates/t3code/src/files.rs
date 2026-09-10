//! The workspace-files half of the T3 compatibility module.
//!
//! Moatless has no directory to point at. A conversation's checkout lives
//! inside its Sandbox, reachable only over that Sandbox's gRPC connection, and
//! the path it sits at is the same string for every conversation. So the `cwd`
//! this module answers is the nominal one it issued as the thread's
//! `worktreePath`, [`workspace::addressed_conversation`] reads the conversation
//! back out of it, and the files come from that conversation's Sandbox.
//!
//! **The listing is whatever the Sandbox walks**, and this is the one caller
//! that asks it to honour the repositories' ignore rules. Presenting a checkout
//! to a person is not the same as reading it: a plain recursive walk of a
//! workspace that has been built in is mostly `target/` and `dist/`, and at
//! this scale that is not clutter but loss — the whole tree arrives in one
//! call, so build output past the cap displaces source that never gets shown.
//! A Sandbox from before that flag existed ignores it and answers as it always
//! did.
//!
//! **A read is addressed from the workspace root, and a mention is not.** The
//! root holds a checkout per repository, so a path an agent writes into a
//! transcript is one directory short of a file here — or, when the agent wrote
//! the path it used inside the Sandbox, a whole disk long. Reading resolves both
//! ([`crate::checkout::nested_match`], [`crate::checkout::rooted_match`]);
//! writing deliberately does not.

use serde_json::{Value, json};
use tracing::warn;

use super::checkout::{Body, Checkout, Sources};
use super::workspace::{self, WorkspaceAccessError};
use crate::reads::T3Reads;
use moatless_ports::sandbox::SandboxError;
use moatless_types::access_control::ScopeContext;
use sandbox_proto::client::{SandboxConnector, classify_grpc_error};

/// The most entries one listing carries.
///
/// Matches the reference server's own index cap, which is where the client's
/// expectations about this call were formed. A Sandbox walk has no equivalent
/// bound of its own — it returns the whole tree, including build output — so
/// without this a repository with a populated `target/` or `.venv` would put
/// several megabytes of paths into a single frame.
const MAX_ENTRIES: usize = 25_000;

/// The most file content one read carries, matching the reference server's cap.
///
/// A file past it comes back cut short with `truncated` set, which is what the
/// client renders as a partially shown file. `byteLength` still reports the
/// whole file, because it is the file's size and not the excerpt's.
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
/// A dot entry is served in the second round, out of what the first leaves. A
/// checkout is always `{workspace_dir}/{mount_name}` (`SandboxConfig::repos`),
/// so a dot entry at the root is what a tool wrote and never a checkout's own
/// `.github/`. Cloning a repo at the root would take that away.
///
/// Smallest group first inside a round, so a share a small entry leaves over
/// goes to the groups still being cut and none of the cap is left unspent.
///
/// A cut group keeps its shallowest paths, ties broken by name, so the client
/// gets the top of that tree and not one deep branch of it. A path's ancestors
/// are shallower than it is, so they reach the listing beside it.
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

    let (dotted, plain): (Vec<usize>, Vec<usize>) = (0..groups.len())
        .partition(|&group| groups[group][0].starts_with('.'));
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
    path.strip_suffix('/')
        .unwrap_or(path)
        .matches('/')
        .count()
}

/// The first segment of `path`, which is the entry the workspace root holds it
/// under. A root-level file is its own segment, and so its own group.
fn top_level(path: &str) -> &str {
    path.split('/').next().unwrap_or(path)
}

/// One entry of a Sandbox listing, in the contract's shape.
///
/// The Sandbox marks a directory by a trailing slash and a file by its absence,
/// which is the whole of the distinction the contract wants. A path that is
/// nothing but that slash is dropped: the contract refuses a blank one, and a
/// listing that fails to decode costs the whole tree rather than the one row.
fn entry(path: &str) -> Option<Value> {
    let (path, kind) = match path.strip_suffix('/') {
        Some(directory) => (directory, "directory"),
        None => (path, "file"),
    };
    (!path.is_empty()).then(|| json!({ "path": path, "kind": kind }))
}

/// Which files here are called something like this, in the shape the
/// contract's `projects.searchEntries` returns.
///
/// Two paths, because the Sandbox answers only one of the two questions this
/// method is asked. A query goes to its name search. **An empty query is a
/// bounded browse** — the contract says so out loud
/// (`packages/contracts/src/project.ts:19`), it is what the file picker opens
/// with, and the Sandbox's walker returns nothing for it
/// (`crates/sandbox/src/files/search.rs:49`) — so that case is answered from the same
/// listing [`list_entries`] reads, shared across the root's entries by
/// [`share_cap`].
///
/// The reference server orders that browse by frecency. Moatless records no
/// per-file access history and a picker's first paint does not justify a table
/// to hold one, so the order is the sorted listing's. A *bounded browse* is all
/// the contract asks for.
///
/// Case-insensitive always: the input carries no case flag, and what reaches
/// here is a person typing a fragment of a name.
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

/// A file written into the conversation's own worktree, in the shape the
/// contract's `projects.writeFile` returns.
///
/// The one write this surface makes outside its own database.
///
/// No conflict detection: the contract's input carries no expected version or
/// hash, so a person and a running agent editing one file race and the last
/// writer wins.
///
/// Path containment is the Sandbox's, as for [`read_file`]. The gate is
/// [`workspace::controlled_conversation`] — `controlled_`, not `addressed_`,
/// because a write puts bytes on someone else's disk, and reading a public
/// conversation's checkout is part of what it shares while editing it is not.
///
/// Do not resolve the path the way [`read_file`] does. A write creates what is
/// missing, so the same rule would turn saving a new root `docs/notes.md` into
/// overwriting a checkout's existing one.
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

/// One file's contents, in the shape the contract's `projects.readFile`
/// returns.
///
/// **The path that comes back is the path that was read**, which is not always
/// the path that was asked for — see [`crate::checkout::nested_match`] and
/// [`crate::checkout::rooted_match`]. The contract carries `relativePath` in the
/// result for exactly this reason, and a client that keeps addressing the file
/// it opened by the path it typed will write its edits somewhere else.
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
/// Cutting a UTF-8 string at a byte offset can land inside a character, and the
/// result would be neither valid JSON string content nor anything the client
/// could render, so the cut moves back to the nearest boundary.
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

    /// One Sandbox, reached at a URL instead of by service name.
    ///
    /// Addressing is the k8s and Docker adapters' own, and neither can be
    /// reached from a test. What is under test here is everything after it:
    /// what a real Sandbox actually returns, and whether this module's
    /// projection of it decodes as the contract's own shapes.
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
            agent_type: moatless_types::agent::AgentType(
                moatless_types::agent::AgentType::CLAUDE_CODE.to_string(),
            ),
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

    /// The whole projection, against a Sandbox that really exists.
    ///
    /// Ignored, because it needs one: only a running deployment has a Sandbox,
    /// and no adapter's addressing is reachable from a test. Point
    /// `T3_UI_RPC_TEST_SANDBOX_GRPC` at one — from inside a sandbox, its own
    /// server is at `http://127.0.0.1:50052` — and run:
    ///
    /// ```text
    /// T3_UI_RPC_TEST_SANDBOX_GRPC=http://127.0.0.1:50052 \
    ///   cargo test -p moatless-t3code --features live-sandbox-tests files -- --ignored --nocapture
    /// ```
    ///
    /// It asserts the two properties a Sandbox's answer has to have to be a
    /// listing at all — every entry carries a relative path the contract
    /// accepts, and the directories a file hangs under are in the listing
    /// beside it — plus that a file named by that listing reads back.
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

    /// The mention case, against a real workspace: a path relative to a
    /// checkout, which the workspace root has no file at, opens the file inside
    /// the checkout and answers with where it was found.
    ///
    /// Ignored for the same reason as the test above, and run the same way.
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
