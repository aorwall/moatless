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

use serde_json::{Value, json};
use tracing::warn;

use super::dao::ConversationDao;
use super::workspace::{self, WorkspaceAccessError};
use crate::sandbox::access::SandboxConnector;
use crate::sandbox::error::SandboxError;
use moatless_backend_core::scope::ScopeContext;

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
    conversations: &dyn ConversationDao,
    sandboxes: &dyn SandboxConnector,
    viewer: &ScopeContext,
    cwd: &str,
) -> Result<Value, WorkspaceAccessError> {
    let task_id = workspace::addressed_conversation(conversations, viewer, cwd).await?;
    // Recursive, because the contract has no second call: the client is given
    // the whole tree once and expands it locally. This is the same shape
    // Moatless's own file view fetches.
    let files = sandboxes
        .sandbox_client(&task_id)
        .await
        .map_err(WorkspaceAccessError::NotRunning)?
        .list_files(".", true, true)
        .await
        .map_err(|status| {
            WorkspaceAccessError::Sandbox(crate::sandbox::client::classify_grpc_error(
                status,
                "list_files",
            ))
        })?;

    let truncated = files.len() > MAX_ENTRIES;
    if truncated {
        warn!(
            task_id = %task_id,
            walked = files.len(),
            cap = MAX_ENTRIES,
            "Sandbox file listing exceeded the T3 UI entry cap; reporting it truncated"
        );
    }
    let entries: Vec<Value> = files
        .iter()
        .take(MAX_ENTRIES)
        .filter_map(|path| entry(path))
        .collect();
    Ok(json!({ "entries": entries, "truncated": truncated }))
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

/// One file's contents, in the shape the contract's `projects.readFile`
/// returns.
pub async fn read_file(
    conversations: &dyn ConversationDao,
    sandboxes: &dyn SandboxConnector,
    viewer: &ScopeContext,
    cwd: &str,
    relative_path: &str,
) -> Result<Value, WorkspaceAccessError> {
    let task_id = workspace::addressed_conversation(conversations, viewer, cwd).await?;
    let file = sandboxes
        .sandbox_client(&task_id)
        .await
        .map_err(WorkspaceAccessError::NotRunning)?
        // Without the git overlay: the contract's result carries contents and
        // nothing else, so asking for a diff would only cost the Sandbox a
        // `git` invocation per opened file.
        .read_file(relative_path, false)
        .await
        .map_err(|status| {
            WorkspaceAccessError::Sandbox(crate::sandbox::client::classify_grpc_error(
                status,
                "read_file",
            ))
        })?;

    if file.is_binary.unwrap_or(false) {
        // The Sandbox base64-encodes what it cannot decode as UTF-8. Returning
        // that as `contents` would put the encoding in the editor as if it were
        // the file, so this is refused in the contract's own terms instead.
        return Err(WorkspaceAccessError::Sandbox(SandboxError::BadRequest(
            format!("{relative_path} is a binary file"),
        )));
    }

    let byte_length = file.content.len();
    let contents = truncate_on_char_boundary(&file.content, MAX_FILE_BYTES);
    Ok(json!({
        "relativePath": relative_path,
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
    struct SandboxAt(String);

    #[async_trait::async_trait]
    impl SandboxConnector for SandboxAt {
        async fn sandbox_client(
            &self,
            _task_id: &str,
        ) -> Result<crate::sandbox::client::SandboxClient, SandboxError> {
            crate::sandbox::client::SandboxClient::connect(&self.0)
                .map_err(SandboxError::GrpcClient)
        }
    }

    /// A conversation with a running Sandbox, and nothing else filled in: the
    /// only fields this module reads are the id and the Sandbox's intent.
    fn running(task_id: &str) -> crate::ui_rpc::dao::Conversation {
        crate::ui_rpc::dao::Conversation {
            task_id: task_id.to_string(),
            workspace_id: Some("ws".into()),
            name: None,
            branch: None,
            agent_type: crate::shared_models::AgentType(
                crate::shared_models::AgentType::CLAUDE_CODE.to_string(),
            ),
            created_at: "2026-01-01T00:00:00Z".into(),
            created_at_db: None,
            updated_at: None,
            turn_number: None,
            turn_status: None,
            turn_started_at: None,
            turn_ended_at: None,
            sandbox_desired_state: Some(moatless_api_schemas::enums::SandboxDesiredState::Running),
            sandbox_stop_reason: None,
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
    ///   cargo test -p moatless-vibe-backend --lib ui_rpc::files -- --ignored --nocapture
    /// ```
    ///
    /// It asserts the two properties a Sandbox's answer has to have to be a
    /// listing at all — every entry carries a relative path the contract
    /// accepts, and the directories a file hangs under are in the listing
    /// beside it — plus that a file named by that listing reads back.
    #[tokio::test]
    #[ignore = "needs a Sandbox at T3_UI_RPC_TEST_SANDBOX_GRPC"]
    async fn a_real_sandbox_listing_becomes_contract_entries() {
        let url = std::env::var("T3_UI_RPC_TEST_SANDBOX_GRPC")
            .expect("point T3_UI_RPC_TEST_SANDBOX_GRPC at a Sandbox's gRPC endpoint");
        let mut conversations = crate::ui_rpc::dao::MockConversationDao::new();
        conversations
            .expect_list_followed_by_ids()
            .returning(|_, ids| Ok(vec![running(&ids[0])]));
        let viewer = ScopeContext {
            user_id: "viewer".into(),
            is_admin: false,
        };
        let cwd = "/moatless/tasks/probe";

        let listing = list_entries(&conversations, &SandboxAt(url.clone()), &viewer, cwd)
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
        let file = read_file(&conversations, &SandboxAt(url), &viewer, cwd, path)
            .await
            .expect("the sandbox answered");
        assert_eq!(file["relativePath"], path);
        assert!(file["contents"].is_string());
        assert!(file["byteLength"].as_u64().is_some());
        println!("read {path}: {} bytes", file["byteLength"]);
    }
}
