use crate::config::AgentModelDefaults;
use crate::dao::Conversation;
use crate::dao::ConversationDao;
use crate::dao::Transcript;
use crate::listing::UiRpcEnvironmentIdentity;
use crate::projection;
use crate::state::Deployment;
use crate::workspace_dao::T3WorkspaceDao;
use moatless_core::change_feed::conversation::ConversationHead;
use moatless_types::access_control::ScopeContext;
use serde_json::Value;
use serde_json::json;
use tracing::info;
use tracing::warn;

/// Assemble the shell listing one viewer sees.
///
/// This is the module's data-access seam, and the whole of its internal
/// interface: read the conversations this viewer follows, read the workspaces
/// they need in order to be grouped, and project both onto the wire.
///
/// The two halves are filtered by deliberately different rules, and the
/// asymmetry is load-bearing rather than an oversight. Conversations are the
/// ones the viewer **follows**, may still read, and that are still **open** — a
/// sidebar is about involvement in unfinished work, so an administrator's is
/// what they follow like everyone else's, and a closed Task leaves it the way an
/// unfollowed one does. See [`dao::ConversationDao::list_followed`].
/// Workspaces are the union of every one the viewer may **read** *plus* every
/// one referenced by a listed conversation, whether or not they may read it:
/// people follow work that runs in someone else's private workspace, and the
/// client silently drops any conversation whose project is missing from the
/// listing, with no error and nothing logged.
pub async fn shell_snapshot(
    workspaces: &dyn T3WorkspaceDao,
    conversations: &dyn ConversationDao,
    model_defaults: &AgentModelDefaults,
    viewer: &ScopeContext,
) -> Result<ShellSnapshot, sqlx::Error> {
    // Before the rows, never after. A position read afterwards would sit above
    // a change the rows already reflect, and the event carrying that change
    // would be discarded by the client as stale — one row frozen, silently.
    // Read first, the worst case is a change delivered twice, and delivery is a
    // whole row, so twice is the same as once.
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
    Ok(ShellSnapshot {
        value: projection::shell_snapshot(&workspaces, &conversations, model_defaults, sequence),
        sequence,
        identity: head.identity,
    })
}

/// One conversation this viewer may read, together with the project a client
/// would file it under — or nothing.
///
/// The project is the second condition, not a detail of the first. A client
/// groups every conversation under a project it already holds, so one whose
/// workspace is not in the listing has nowhere to go and is dropped here rather
/// than sent to be silently discarded on arrival.
///
/// `Ok(None)` collapses every way a conversation can be absent for this viewer
/// into one answer; distinguishing them would disclose which.
pub(crate) async fn readable_under_a_project(
    workspaces: &dyn T3WorkspaceDao,
    conversations: &dyn ConversationDao,
    viewer: &ScopeContext,
    task_id: &str,
) -> Result<Option<(Conversation, String)>, sqlx::Error> {
    let Some(conversation) = conversations.readable_by_id(viewer, task_id).await? else {
        return Ok(None);
    };
    let referenced: Vec<String> = conversation.workspace_id.clone().into_iter().collect();
    let workspaces = workspaces
        .list_readable_including(viewer, &referenced)
        .await?;
    let project_id = conversation
        .workspace_id
        .as_deref()
        .filter(|id| workspaces.iter().any(|listed| listed.workspace.id == *id))
        .map(str::to_string);
    let Some(project_id) = project_id else {
        // The shell drops this conversation for the same reason, with the same
        // warning. Opening one the sidebar could not show it is the same gap
        // seen from the other side.
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
/// The listings answer "what am I involved in"; this answers "what is this
/// one", and those are different questions. A conversation reaches a client
/// through a link, a task tree or an archive long before it reaches the
/// sidebar — closed work never reaches the sidebar at all — and without this
/// the client holds a thread it can render a transcript for and cannot name.
///
/// Resolved by access rather than by Follow, exactly as [`thread_snapshot`] is:
/// the two are the same conversation seen at two levels of detail, so a
/// deployment where one opens and the other does not would be the bug.
pub async fn thread_shell(
    workspaces: &dyn T3WorkspaceDao,
    conversations: &dyn ConversationDao,
    defaults: &AgentModelDefaults,
    viewer: &ScopeContext,
    task_id: &str,
) -> Result<Option<Value>, sqlx::Error> {
    Ok(
        readable_under_a_project(workspaces, conversations, viewer, task_id)
            .await?
            .map(|(conversation, project_id)| {
                projection::thread_shell(&conversation, &project_id, defaults)
            }),
    )
}

/// Assemble one conversation as the thread view carries it.
///
/// `Ok(None)` collapses every way a conversation can be absent for this viewer
/// into one answer; distinguishing them would disclose which.
///
/// The row is resolved by access ([`ConversationDao::readable_by_id`]), not by
/// Follow, so a link to a readable Task opens. The sidebar stays narrowed to
/// Follows until the reader writes. Reading is the whole of that widening —
/// running a turn resolves through [`commands::require_controllable`].
///
/// Read the change position **before** the rows, as [`shell_snapshot`] does:
/// read afterwards it sits above a change the rows already reflect and the
/// client discards that event as stale, freezing the conversation. Read first,
/// the worst case is one change delivered twice, and every event here is an
/// upsert.
pub async fn thread_snapshot(
    workspaces: &dyn T3WorkspaceDao,
    conversations: &dyn ConversationDao,
    deployment: &Deployment<'_>,
    viewer: &ScopeContext,
    task_id: &str,
) -> Result<Option<ThreadSnapshot>, sqlx::Error> {
    let head = conversations.conversation_head(task_id).await?;
    let Some((conversation, project_id)) =
        readable_under_a_project(workspaces, conversations, viewer, task_id).await?
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
    Ok(Some(ThreadSnapshot {
        value: json!({
            "snapshotSequence": head.sequence(),
            "thread": projection::thread_detail(
                &conversation,
                project_id,
                deployment.model_defaults,
                &transcript,
                &projection::OriginContext {
                    frontend_url: deployment.frontend_url,
                    source_task_names: &source_task_names(conversations, viewer, &transcript).await,
                },
            )
        }),
        head,
    }))
}

/// The names of the Tasks a window's Messages came from, by task id.
///
/// One scoped read, and only when a Message in the window came from another
/// Task at all — which nearly none do. A failure here costs the origins their
/// label and their link; it must not cost the reader the conversation, so it is
/// logged and answered with nothing.
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

/// One conversation as the client receives it, and the position it was built
/// at.
///
/// They travel together because a subscription needs both: the conversation
/// goes to the client and the position is where its feed starts watching.
pub struct ThreadSnapshot {
    pub value: Value,
    pub head: ConversationHead,
}

/// A shell listing, the position it was read at, and the database it was read
/// from.
///
/// They travel together because a subscription needs all three: the listing goes
/// to the client, and the position and identity are where its live feed starts.
/// Splitting them is what lets the second be taken after the first, which is the
/// ordering mistake this whole mechanism exists to avoid.
pub struct ShellSnapshot {
    pub value: Value,
    pub sequence: i64,
    pub identity: UiRpcEnvironmentIdentity,
}

/// The workspace root a file request addresses.
///
/// The contract types `cwd` as a trimmed non-empty string, so a payload
/// carrying anything else did not come from a client encoding against it. It is
/// refused rather than defaulted: every value this module could substitute
/// names somebody's conversation.
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
/// The contract types `afterSequence` as an optional `NonNegativeInt`, so a
/// value that is neither absent nor a non-negative integer cannot come from a
/// client that encodes against it. Reading one as *absent* is the safe
/// direction: the client is rebuilt, which is a listing it can use. Reading it
/// as a position would be resuming from a number nothing allocated.
pub(crate) fn after_sequence(payload: &Value) -> Option<i64> {
    payload
        .get("afterSequence")
        .and_then(Value::as_i64)
        .filter(|sequence| *sequence >= 0)
}

impl ShellSnapshot {
    /// The conversation ids this listing puts in front of the viewer.
    ///
    /// Read back out of the projected listing rather than off the rows it was
    /// built from, and deliberately: a conversation whose project is missing is
    /// dropped during projection, and a feed that believed the client held it
    /// would later "remove" a row that was never on the screen. What was sent is
    /// the only honest answer to what is held.
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

/// The archived half of the listing: the closed conversations the viewer
/// follows, under the same projects the live half carries.
///
/// Archiving is closing, so this is the live listing's complement — the same
/// follow and read rules at the other status. The projects come whole rather
/// than narrowed to the ones these conversations sit in, because the client
/// groups an archived row under a project it must already hold.
pub async fn archived_shell_snapshot(
    workspaces: &dyn T3WorkspaceDao,
    conversations: &dyn ConversationDao,
    defaults: &AgentModelDefaults,
    viewer: &ScopeContext,
) -> Result<Value, sqlx::Error> {
    let sequence = conversations.listing_head().await?.position;
    let workspaces = workspaces.list_readable_including(viewer, &[]).await?;
    let archived = conversations.list_followed_closed(viewer).await?;
    Ok(projection::shell_snapshot(
        &workspaces,
        &archived,
        defaults,
        sequence,
    ))
}
