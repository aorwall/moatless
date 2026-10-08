use crate::config::AgentModelDefaults;
use crate::config::OrchestrationProtocol;
use crate::dao::ArchiveCursor;
use crate::dao::Conversation;
use crate::dao::ConversationBrowse;
use crate::dao::ConversationDao;
use crate::lineage;
use crate::listing::UiRpcEnvironmentIdentity;
use crate::ports::TaskRelationSource;
use crate::projection;
use crate::reads::T3Reads;
use crate::state::Deployment;
use crate::transcript_dao::Transcript;
use moatless_core::change_feed::conversation::ConversationHead;
use moatless_types::access_control::ScopeContext;
use serde_json::Value;
use serde_json::json;
use tracing::info;
use tracing::warn;

/// Assemble the shell listing one viewer sees.
///
/// Conversations are followed, readable, and open; a conversation whose
/// workspace is missing from the viewer's readable set is silently dropped.
pub async fn shell_snapshot(
    reads: &T3Reads,
    model_defaults: &AgentModelDefaults,
    viewer: &ScopeContext,
) -> Result<ShellSnapshot, sqlx::Error> {
    shell_snapshot_in(reads, model_defaults, viewer, OrchestrationProtocol::V1).await
}

/// [`shell_snapshot`], in the wire shape `protocol` names. V2's
/// `archivedThreads` is always empty: the archive is its own paged listing.
pub(crate) async fn shell_snapshot_in(
    reads: &T3Reads,
    model_defaults: &AgentModelDefaults,
    viewer: &ScopeContext,
    protocol: OrchestrationProtocol,
) -> Result<ShellSnapshot, sqlx::Error> {
    let conversations = reads.conversations();
    let workspaces = reads.workspaces();
    // Read position before rows: reading after risks the client discarding a
    // live change as stale. Read first, worst case is one change delivered twice.
    let head = conversations.listing_head().await?;
    let sequence = head.position;
    let conversations = conversations.list_followed(viewer).await?;
    let referenced_workspace_ids: Vec<String> = conversations
        .iter()
        .filter_map(|conversation| conversation.workspace_id.clone())
        .collect();
    let workspaces = workspaces
        .list_readable_including(viewer, &referenced_workspace_ids)
        .await?;
    let value = match protocol {
        OrchestrationProtocol::V1 => {
            projection::shell_snapshot(&workspaces, &conversations, model_defaults, sequence)
        }
        OrchestrationProtocol::V2 => projection::shell_snapshot_v2(
            &workspaces,
            &conversations,
            &[],
            model_defaults,
            sequence,
        ),
    };
    Ok(ShellSnapshot {
        value,
        sequence,
        identity: head.identity,
    })
}

/// One conversation this viewer may read, with the project a client would file
/// it under — or nothing.
///
/// `Ok(None)` collapses every way it can be absent into one answer;
/// distinguishing them would disclose which.
pub(crate) async fn readable_under_a_project(
    reads: &T3Reads,
    viewer: &ScopeContext,
    task_id: &str,
) -> Result<Option<(Conversation, String)>, sqlx::Error> {
    let Some(conversation) = reads
        .conversations()
        .readable_by_id(viewer, task_id)
        .await?
    else {
        return Ok(None);
    };
    let referenced: Vec<String> = conversation.workspace_id.clone().into_iter().collect();
    let workspaces = reads
        .workspaces()
        .list_readable_including(viewer, &referenced)
        .await?;
    let project_id = conversation
        .workspace_id
        .as_deref()
        .filter(|id| workspaces.iter().any(|listed| listed.workspace.id == *id))
        .map(str::to_string);
    let Some(project_id) = project_id else {
        // Mirrors the shell's drop of the same conversation, with the same warning.
        warn!(
            task_id = %conversation.task_id,
            workspace_id = ?conversation.workspace_id,
            "Conversation has no listable workspace; not opening it in the T3 UI"
        );
        return Ok(None);
    };
    Ok(Some((conversation, project_id)))
}

/// One conversation's listing row, by id.
///
/// Resolved by access, not Follow, like [`thread_snapshot`]: the two are the
/// same conversation at two levels of detail, so they must open together.
pub async fn thread_shell(
    reads: &T3Reads,
    defaults: &AgentModelDefaults,
    viewer: &ScopeContext,
    task_id: &str,
    protocol: OrchestrationProtocol,
) -> Result<Option<Value>, sqlx::Error> {
    Ok(readable_under_a_project(reads, viewer, task_id)
        .await?
        .map(|(conversation, project_id)| match protocol {
            OrchestrationProtocol::V1 => {
                projection::thread_shell(&conversation, &project_id, defaults)
            }
            OrchestrationProtocol::V2 => projection::thread_shell_v2(
                &conversation,
                &project_id,
                defaults,
                &projection::Lineage::listed(&conversation, &Default::default()),
            ),
        }))
}

/// Assemble one conversation as the thread view carries it.
///
/// `Ok(None)` collapses every absence into one answer. Reads the change
/// position before the rows, like [`shell_snapshot`], to avoid discarding a
/// live change.
pub async fn thread_snapshot(
    reads: &T3Reads,
    deployment: &Deployment<'_>,
    viewer: &ScopeContext,
    task_id: &str,
) -> Result<Option<ThreadSnapshot>, sqlx::Error> {
    let conversations = reads.conversations();
    let head = conversations.conversation_head(task_id).await?;
    let Some((conversation, project_id)) = readable_under_a_project(reads, viewer, task_id).await?
    else {
        return Ok(None);
    };
    let project_id = project_id.as_str();
    let transcript = conversations.transcript(task_id).await?;
    if transcript.omitted > 0 {
        info!(
            task_id,
            omitted = transcript.omitted,
            "T3 UI thread snapshot left out the oldest messages of a long conversation"
        );
    }
    let thread = projection::thread_detail(
        &conversation,
        project_id,
        deployment.model_defaults,
        &transcript,
        &projection::OriginContext {
            frontend_url: deployment.frontend_url,
            source_task_names: &source_task_names(conversations, viewer, &transcript).await,
        },
    );
    let checkpointed = thread["checkpoints"]
        .as_array()
        .and_then(|checkpoints| checkpoints.last())
        .and_then(|checkpoint| checkpoint["checkpointTurnCount"].as_i64())
        .and_then(|turn| i32::try_from(turn).ok());
    Ok(Some(ThreadSnapshot {
        value: json!({
            "snapshotSequence": head.sequence(),
            "thread": thread
        }),
        head,
        checkpointed,
    }))
}

/// One conversation as an `OrchestrationV2ThreadDetailSnapshot`, with the
/// position it was built at.
///
/// The whole transcript window, never a bounded page: `hasMoreHistory` is
/// always `false`, so the client never asks for an older page this server
/// has no route for.
pub(crate) async fn thread_detail_v2(
    reads: &T3Reads,
    deployment: &Deployment<'_>,
    relations: &dyn TaskRelationSource,
    viewer: &ScopeContext,
    task_id: &str,
) -> Result<Option<ThreadDetailV2>, sqlx::Error> {
    let lineage = lineage::resolve(reads, relations, viewer, task_id).await;
    thread_detail_v2_under(reads, deployment, viewer, task_id, lineage).await
}

/// [`thread_detail_v2`] with the thread's lineage already resolved, which never
/// changes for a thread once it exists.
pub(crate) async fn thread_detail_v2_under(
    reads: &T3Reads,
    deployment: &Deployment<'_>,
    viewer: &ScopeContext,
    task_id: &str,
    lineage: projection::Lineage,
) -> Result<Option<ThreadDetailV2>, sqlx::Error> {
    let conversations = reads.conversations();
    let head = conversations.conversation_head(task_id).await?;
    let Some((conversation, project_id)) = readable_under_a_project(reads, viewer, task_id).await?
    else {
        return Ok(None);
    };
    let transcript = conversations.transcript(task_id).await?;
    let projection = projection::thread_projection(
        &conversation,
        &project_id,
        deployment.model_defaults,
        &transcript,
        &projection::OriginContext {
            frontend_url: deployment.frontend_url,
            source_task_names: &source_task_names(conversations, viewer, &transcript).await,
        },
        &lineage,
    );
    Ok(Some(ThreadDetailV2 {
        snapshot: json!({
            "snapshotSequence": head.sequence(),
            "projection": projection,
            "historyCursor": Value::Null,
            "hasMoreHistory": false
        }),
        head,
        lineage,
    }))
}

/// [`thread_detail_v2`]'s answer.
pub(crate) struct ThreadDetailV2 {
    /// The `OrchestrationV2ThreadDetailSnapshot`: `snapshotSequence`,
    /// `projection` and the history keys.
    pub(crate) snapshot: Value,
    pub(crate) head: ConversationHead,
    pub(crate) lineage: projection::Lineage,
}

/// The names of the Tasks a window's Messages came from, by task id.
///
/// A failure here must not cost the reader the conversation: logged, and
/// answered with an empty map instead of propagated.
pub(crate) async fn source_task_names(
    conversations: &dyn ConversationDao,
    viewer: &ScopeContext,
    window: &Transcript,
) -> std::collections::HashMap<String, String> {
    let mut ids: Vec<String> = window
        .messages
        .iter()
        .filter_map(|message| message.source_task_id.clone())
        .collect();
    if ids.is_empty() {
        return std::collections::HashMap::new();
    }
    ids.sort();
    ids.dedup();
    match conversations.source_task_names(viewer, &ids).await {
        Ok(names) => names,
        Err(error) => {
            warn!(%error, "T3 UI could not resolve the names of source tasks; origins go unlabelled");
            std::collections::HashMap::new()
        }
    }
}

/// One conversation as the client receives it, with the position it was built
/// at — a subscription needs both, since the position is where its feed starts.
pub struct ThreadSnapshot {
    pub value: Value,
    pub head: ConversationHead,
    /// The newest turn `value` carries a checkpoint for.
    pub checkpointed: Option<i32>,
}

/// A shell listing, the position it was read at, and the database it was read
/// from. Bundled together so a caller can't split them and read the position in
/// the wrong order relative to the listing.
pub struct ShellSnapshot {
    pub value: Value,
    pub sequence: i64,
    pub identity: UiRpcEnvironmentIdentity,
}

/// The workspace root a file request addresses.
///
/// Refused, not defaulted: any substitute for a missing `cwd` would name
/// somebody else's conversation.
pub(crate) fn workspace_cwd(payload: &Value) -> Option<String> {
    payload
        .get("cwd")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|cwd| !cwd.is_empty())
        .map(str::to_owned)
}

/// The position a client says it already holds.
///
/// Invalid input reads as absent, not a position: rebuilding the client is
/// safe, resuming from an unallocated number is not.
pub(crate) fn after_sequence(payload: &Value) -> Option<i64> {
    payload
        .get("afterSequence")
        .and_then(Value::as_i64)
        .filter(|sequence| *sequence >= 0)
}

impl ShellSnapshot {
    /// The conversation ids this listing puts in front of the viewer.
    ///
    /// Read from the projected listing, not the source rows: a row dropped
    /// during projection was never sent, so a feed off the rows would try to
    /// remove one the client never held.
    pub fn thread_ids(&self) -> Vec<String> {
        self.ids("threads", "id")
    }

    /// The project ids this listing puts in front of the viewer.
    pub fn project_ids(&self) -> Vec<String> {
        self.ids("projects", "id")
    }

    pub(crate) fn ids(&self, collection: &str, key: &str) -> Vec<String> {
        self.value[collection]
            .as_array()
            .map(|rows| {
                rows.iter()
                    .filter_map(|row| row[key].as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// The listing rows a browse filter names — conversations the viewer may read,
/// not necessarily follows.
///
/// No sequence travels with these; a client re-asks to stay current.
/// `truncated` is decided on the raw rows, before projection can drop any.
pub async fn browse_snapshot(
    reads: &T3Reads,
    defaults: &AgentModelDefaults,
    viewer: &ScopeContext,
    filter: &ConversationBrowse,
    protocol: OrchestrationProtocol,
) -> Result<Value, sqlx::Error> {
    let mut conversations = reads.conversations().list_browsable(viewer, filter).await?;
    let truncated = conversations.len() as i64 > crate::dao::BROWSE_LIMIT;
    conversations.truncate(crate::dao::BROWSE_LIMIT as usize);
    let workspaces = reads
        .workspaces()
        .list_readable_including(viewer, &[])
        .await?;
    let threads = match protocol {
        OrchestrationProtocol::V1 => {
            projection::listed_threads(&workspaces, &conversations, defaults)
        }
        OrchestrationProtocol::V2 => {
            projection::listed_threads_v2(&workspaces, &conversations, defaults)
        }
    };
    Ok(json!({
        "threads": threads,
        "truncated": truncated,
    }))
}

/// The newest page of the archived half of the listing: see
/// [`archived_shell_page`].
pub async fn archived_shell_snapshot(
    reads: &T3Reads,
    defaults: &AgentModelDefaults,
    viewer: &ScopeContext,
) -> Result<Value, sqlx::Error> {
    archived_shell_page(reads, defaults, viewer, None, OrchestrationProtocol::V1).await
}

/// One page of closed conversations the viewer follows, newest first.
///
/// At most [`crate::dao::ARCHIVED_LIMIT`], older than `before`; `nextCursor`
/// resumes after the page and is `null` on the last. Projects come whole: the
/// client groups an archived row under a project it must already hold.
pub(crate) async fn archived_shell_page(
    reads: &T3Reads,
    defaults: &AgentModelDefaults,
    viewer: &ScopeContext,
    before: Option<ArchiveCursor>,
    protocol: OrchestrationProtocol,
) -> Result<Value, sqlx::Error> {
    let conversations = reads.conversations();
    let workspaces = reads.workspaces();
    let sequence = conversations.listing_head().await?.position;
    let workspaces = workspaces.list_readable_including(viewer, &[]).await?;
    let mut archived = conversations.list_followed_closed(viewer, before).await?;
    let truncated = archived.len() as i64 > crate::dao::ARCHIVED_LIMIT;
    archived.truncate(crate::dao::ARCHIVED_LIMIT as usize);
    // Taken from the raw rows, as `truncated` is in `browse_snapshot`: a row
    // projection drops still holds its place in the order.
    let next_cursor = archived
        .last()
        .filter(|_| truncated)
        .map(|last| ArchiveCursor::after(last).encode());
    let mut snapshot = match protocol {
        OrchestrationProtocol::V1 => {
            projection::shell_snapshot(&workspaces, &archived, defaults, sequence)
        }
        OrchestrationProtocol::V2 => {
            projection::archived_shell_snapshot_v2(&workspaces, &archived, defaults, sequence)
        }
    };
    snapshot["nextCursor"] = json!(next_cursor);
    Ok(snapshot)
}
