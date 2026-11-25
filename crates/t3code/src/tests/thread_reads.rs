//! Find in a thread and the title lookups.

use super::fixtures::*;
use super::*;
use crate::dao::Conversation;
use crate::dao::running_conversation;
use moatless_core::change_feed::conversation::ConversationHead;
use moatless_tasks::conversation::transcript::TranscriptEntry;
use moatless_types::message::{MessageRole, MessageType};
use moatless_workspaces::workspace::models::ListedWorkspace;

fn conversation(task_id: &str, name: Option<&str>, workspace_id: &str) -> Conversation {
    let mut conversation = running_conversation(task_id);
    conversation.name = name.map(str::to_string);
    conversation.workspace_id = Some(workspace_id.to_string());
    conversation
}

fn listed(workspace_id: &str) -> MockWorkspaceListing {
    let mut workspace =
        moatless_workspaces::test_support::make_workspace_row(workspace_id, "user-1");
    workspace.id = workspace_id.to_string();
    let listed = vec![ListedWorkspace {
        readable: true,
        workspace,
    }];
    let mut workspaces = MockWorkspaceListing::new();
    workspaces
        .expect_list_readable_including()
        .returning(move |_, _| Ok(listed.clone()));
    workspaces
}

fn entry(id: i32, kind: MessageType, text: &str) -> TranscriptEntry {
    TranscriptEntry {
        id,
        uuid: format!("msg-{id}"),
        message_type: kind,
        message_role: if kind == MessageType::User {
            MessageRole::User
        } else {
            MessageRole::Assistant
        },
        message_text: Some(text.to_string()),
        user_type: "human".to_string(),
        sandbox_wake: false,
        turn_number: Some(id),
        held: false,
        queue_order: None,
        timestamp: "2026-07-01T10:00:00.000Z".to_string(),
        created_at: None,
        external_adapter: None,
        external_source_name: None,
        external_subject_key: None,
        external_event_id: None,
        external_author: None,
        source_task_id: None,
        sent_by: None,
        file_contexts: None,
        child_outcome: None,
        harness: None,
        model: None,
        effort: None,
    }
}

/// `task-1` under a listed Workspace, whose Messages are `messages`.
fn thread_with(messages: Vec<TranscriptEntry>) -> UiRpcState {
    let mut conversations = MockConversationDao::new();
    conversations
        .expect_conversation_head()
        .returning(|_| Ok(ConversationHead { position: 7 }));
    conversations
        .expect_readable_by_id()
        .returning(|_, task_id| {
            Ok((task_id == "task-1").then(|| conversation(task_id, None, "ws-1")))
        });
    conversations
        .expect_messages_after()
        .returning(move |_, after, limit| {
            Ok(messages
                .iter()
                .filter(|entry| after.is_none_or(|after| entry.id > after))
                .take(usize::try_from(limit).unwrap_or(0))
                .cloned()
                .collect())
        });
    state_reading(conversations, listed("ws-1"))
}

fn find(payload: Value) -> Vec<u8> {
    request("1", "orchestration.searchThread", payload)
}

#[tokio::test]
async fn find_counts_each_rendered_occurrence_and_selects_the_first() {
    let state = thread_with(vec![
        entry(1, MessageType::User, "Fix the **build** please"),
        entry(
            2,
            MessageType::Assistant,
            "The [build](https://build.example) broke; BUILD again.",
        ),
        entry(3, MessageType::Assistant, "Nothing here."),
    ]);
    let output = connection()
        .exchange(
            &find(json!({ "threadId": "task-1", "query": "build" })),
            &state,
            &viewer(),
        )
        .await;

    let result = &output.frames[0]["exit"]["value"];
    assert_eq!(result["totalMatches"], 3);
    assert_eq!(result["activeIndex"], 0);
    assert_eq!(
        result["match"],
        json!({ "entryId": "msg-1", "runId": "task-1:1", "occurrence": 0 })
    );
    assert_eq!(
        result["navigation"],
        json!([
            { "entryId": "msg-1", "runId": "task-1:1", "startIndex": 0, "count": 1 },
            { "entryId": "msg-2", "runId": "task-1:2", "startIndex": 1, "count": 2 },
        ])
    );
}

/// A conversation longer than one page is read to its end, and a step past
/// the last occurrence wraps to the first.
#[tokio::test]
async fn find_pages_through_a_long_conversation_and_wraps_a_step() {
    let messages: Vec<TranscriptEntry> = (1..=450)
        .map(|id| entry(id, MessageType::Assistant, "needle"))
        .collect();
    let state = thread_with(messages);
    let output = connection()
        .exchange(
            &find(json!({
                "threadId": "task-1",
                "query": "needle",
                "start": { "entryId": "msg-450", "occurrence": 0 },
                "offset": 1,
            })),
            &state,
            &viewer(),
        )
        .await;

    let result = &output.frames[0]["exit"]["value"];
    assert_eq!(result["totalMatches"], 450);
    assert_eq!(result["activeIndex"], 0);
    assert_eq!(result["match"]["entryId"], "msg-1");
    assert_eq!(result["navigation"].as_array().map(Vec::len), Some(17));
}

#[tokio::test]
async fn find_in_a_thread_the_viewer_cannot_read_is_the_typed_error() {
    let output = connection()
        .exchange(
            &find(json!({ "threadId": "task-2", "query": "x" })),
            &thread_with(Vec::new()),
            &viewer(),
        )
        .await;

    let failure = &output.frames[0]["exit"]["cause"][0]["error"];
    assert_eq!(failure["_tag"], "OrchestrationV2SearchThreadError");
}

#[tokio::test]
async fn the_progressive_find_sends_the_first_occurrence_then_the_count() {
    let state = thread_with(vec![
        entry(1, MessageType::User, "needle"),
        entry(2, MessageType::Assistant, "needle needle"),
    ]);
    let mut connection = connection();
    connection
        .exchange(
            &request(
                "1",
                "orchestration.searchThreadStream",
                json!({ "threadId": "task-1", "query": "needle" }),
            ),
            &state,
            &viewer(),
        )
        .await;

    let first = connection.next_streamed().await;
    assert_eq!(first["values"][0]["complete"], false);
    assert_eq!(first["values"][0]["match"]["entryId"], "msg-1");
    let last = connection.next_streamed().await;
    assert_eq!(last["values"][0]["totalMatches"], 3);
    assert!(last["values"][0].get("complete").is_none());
    let exit = connection.next_streamed().await;
    assert_eq!(exit["_tag"], "Exit");
}

fn titled(conversations: Vec<Conversation>) -> UiRpcState {
    let mut dao = MockConversationDao::new();
    dao.expect_readable_by_ids()
        .returning(move |_, _| Ok(conversations.clone()));
    state_reading(dao, listed("ws-1"))
}

/// What `readable_by_ids` leaves out — another viewer's Task, a deleted one —
/// is absent from the answer, which is how a deleted fork leaves Lineage.
#[tokio::test]
async fn titles_name_what_the_viewer_can_read_and_omit_the_rest() {
    let state = titled(vec![
        conversation("task-1", Some("Fix the build"), "ws-1"),
        conversation("task-2", None, "ws-1"),
    ]);
    let output = connection()
        .exchange(
            &request(
                "1",
                "orchestration.getThreadTitles",
                json!({ "threadIds": ["task-1", "task-2", "gone"] }),
            ),
            &state,
            &viewer(),
        )
        .await;

    assert_eq!(
        output.frames[0]["exit"]["value"],
        json!({ "task-1": "Fix the build", "task-2": "New thread" })
    );
}

#[tokio::test]
async fn link_metadata_names_the_project_only_where_it_is_listed() {
    let state = titled(vec![
        conversation("task-1", Some("Fix the build"), "ws-1"),
        conversation("task-2", Some("Elsewhere"), "ws-hidden"),
    ]);
    let output = connection()
        .exchange(
            &request(
                "1",
                "orchestration.getThreadLinkMetadata",
                json!({ "threadIds": ["task-1", "task-2"] }),
            ),
            &state,
            &viewer(),
        )
        .await;

    assert_eq!(
        output.frames[0]["exit"]["value"],
        json!({
            "task-1": { "title": "Fix the build", "projectId": "ws-1" },
            "task-2": { "title": "Elsewhere", "projectId": null },
        })
    );
}

/// The three keys a client reads before it asks any of these methods.
#[tokio::test]
async fn the_config_reports_find_and_link_metadata() {
    let config = server_config(&state(), &viewer(), &[]).await;
    assert_eq!(config["threadFind"], true);
    assert_eq!(config["threadFindProgressive"], true);
    assert_eq!(config["threadLinkMetadata"], true);
}
