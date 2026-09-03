#![cfg(feature = "db-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr
)]
//! What a T3 command actually writes.
//!
//! `t3code::commands` is unit-tested against a recording sink, so the dispatch
//! table — which commands are accepted, which are refused, how a model slug is
//! translated — is covered without a database. What that cannot see is the far
//! side of the port: whether the client's message id survives into
//! `task_messages`, whether the unique index turns a replayed frame into one
//! row, whether a closed Task refuses, and whether a Thread created with
//! nothing in it stays without a Sandbox. All four are database facts, so they
//! are asserted here, through `AppStateCommands` — the same implementation
//! `main` wires into the socket.

use crate::common;

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use moatless_t3code::ports::{
    AttachmentKind, AttachmentStore, CommandRefusal, ForkThread, NewThread, ThreadCommandSink,
    ThreadMeta, ThreadTurn,
};
use moatless_t3code::{thread_snapshot, AgentModelDefaults, Deployment};
use moatless_backend::agent::catalog;
use moatless_backend::repository::dao as repository_dao;
use moatless_types::access_control::ScopeContext;
use moatless_types::agent::{AgentMode, AgentType};
use moatless_backend::t3code::backend::{AppStateAttachments, AppStateCommands};
use moatless_backend::task::dao::TaskDao;
use moatless_backend::task::models::{TaskStatus, Visibility};
use moatless_backend::workspace::dao::{PgWorkspaceDao, WorkspaceDao};
use sqlx::PgPool;

use common::{
    make_repository_row, make_task_row, make_workspace_repo_row, make_workspace_row,
    test_app_state, USER_1, USER_2,
};

const WS: &str = "ws-t3";

fn viewer(user_id: &str) -> ScopeContext {
    ScopeContext {
        user_id: user_id.into(),
        is_admin: false,
    }
}

fn defaults() -> AgentModelDefaults {
    AgentModelDefaults {
        claude_code: Some("claude-opus-4-5".into()),
        codex: Some("gpt-5.6-sol".into()),
    }
}

/// The deployment a snapshot is projected against. Nothing here asserts on the
/// links built from `frontend_url`, so it carries the development origin.
fn deployment(model_defaults: &AgentModelDefaults) -> Deployment<'_> {
    Deployment {
        model_defaults,
        frontend_url: "http://localhost:3000",
    }
}

fn sink(pool: &PgPool) -> AppStateCommands {
    AppStateCommands(test_app_state(pool.clone()))
}

/// A workspace with a primary repository behind it — creation resolves one, and
/// a Workspace with none is a refusal rather than the case under test.
async fn seed_project(pool: &PgPool, owner: &str) {
    let mut repo = make_repository_row("repo-1", owner);
    // Creation pre-flights a GitHub credential for a GitHub-backed repository,
    // and no test here has one. Whether a task can reach its remote is asserted
    // in the sandbox pre-flight's own tests; what these need is a repository
    // creation resolves.
    repo.provider = None;
    repository_dao::insert(pool, &repo).await.unwrap();
    let workspaces = PgWorkspaceDao(pool.clone());
    workspaces
        .insert_workspace(&make_workspace_row(WS, owner))
        .await
        .unwrap();
    workspaces
        .add_repo(
            &make_workspace_repo_row("wsr-1", WS, "repo-1", true, 0),
            &viewer(owner),
        )
        .await
        .unwrap();
}

/// A second project, for the conversation a sender must not be able to reach.
async fn seed_project_for(pool: &PgPool, owner: &str) {
    let mut repo = make_repository_row("repo-2", owner);
    repo.provider = None;
    repository_dao::insert(pool, &repo).await.unwrap();
    let workspaces = PgWorkspaceDao(pool.clone());
    workspaces
        .insert_workspace(&make_workspace_row("ws-other", owner))
        .await
        .unwrap();
    workspaces
        .add_repo(
            &make_workspace_repo_row("wsr-2", "ws-other", "repo-2", true, 0),
            &viewer(owner),
        )
        .await
        .unwrap();
}

async fn seed_task_owned(pool: &PgPool, task_id: &str, owner: &str) {
    let mut task = make_task_row(task_id, owner);
    task.workspace_id = Some("ws-other".to_string());
    moatless_backend::task::dao::store_over(pool).insert_task(&task).await.unwrap();
}

async fn seed_task(pool: &PgPool, task_id: &str, owner: &str) {
    let mut task = make_task_row(task_id, owner);
    task.workspace_id = Some(WS.to_string());
    moatless_backend::task::dao::store_over(pool).insert_task(&task).await.unwrap();
}

async fn seed_public_task(pool: &PgPool, task_id: &str, owner: &str) {
    let mut task = make_task_row(task_id, owner);
    task.workspace_id = Some(WS.to_string());
    task.visibility = Visibility::Public;
    moatless_backend::task::dao::store_over(pool).insert_task(&task).await.unwrap();
}

async fn follow(pool: &PgPool, user_id: &str, task_id: &str) {
    moatless_backend::task::dao::store_over(pool)
        .follow_task(user_id, task_id)
        .await
        .unwrap();
}

async fn follow_state(
    pool: &PgPool,
    user_id: &str,
    task_id: &str,
) -> (
    Option<String>,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
) {
    sqlx::query_as(
        r#"SELECT settled_override, settled_at, snoozed_until, snoozed_at
           FROM task_follows
           WHERE user_id = $1 AND task_id = $2"#,
    )
    .bind(user_id)
    .bind(task_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// The Sandbox intent the lifecycle persisted, or `None` while no operation has
/// ever asked for one.
async fn sandbox_intent(pool: &PgPool, task_id: &str) -> Option<(String, Option<String>)> {
    sqlx::query_as(
        r#"SELECT s.desired_state, s.stop_reason
           FROM tasks t
           JOIN sandboxes s ON s.sandbox_id = COALESCE(t.sandbox_id, t.task_id)
           WHERE t.task_id = $1"#,
    )
    .bind(task_id)
    .fetch_optional(pool)
    .await
    .unwrap()
}

async fn listing_change_count(pool: &PgPool, task_id: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM task_listing_changes WHERE task_id = $1")
        .bind(task_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// What the person wrote, which is the Message itself now — text lives on the
/// row, so there is no block list to reassemble it from.
async fn message_text(pool: &PgPool, uuid: &str) -> Option<String> {
    sqlx::query_scalar("SELECT message_text FROM task_messages WHERE uuid = $1")
        .bind(uuid)
        .fetch_optional(pool)
        .await
        .unwrap()
        .flatten()
}

fn turn(task_id: &str, uuid: &str, text: &str) -> ThreadTurn {
    ThreadTurn {
        task_id: task_id.into(),
        text: text.into(),
        message_uuid: uuid.into(),
        model: None,
        agent_mode: None,
        effort: None,
        attachments: Vec::new(),
    }
}

/// The uuids of the messages a person wrote in a Task, oldest first.
///
/// Only `user` rows: the test runtime cannot start a Sandbox, so every send
/// also persists the error result that failure produces. That row is a
/// deliberate part of the send path — asserted in `message::send`'s own tests —
/// and counting it here would only measure the test runtime.
async fn message_uuids(pool: &PgPool, task_id: &str) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT uuid FROM task_messages
         WHERE task_id = $1 AND message_type = 'user'
         ORDER BY id",
    )
    .bind(task_id)
    .fetch_all(pool)
    .await
    .unwrap()
}

/// The upload ids a person's message carries, in the order they were attached.
async fn message_file_ids(pool: &PgPool, task_id: &str) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT reference_id
         FROM task_message_contexts
         WHERE task_id = $1 AND type = 'file'
         ORDER BY id",
    )
    .bind(task_id)
    .fetch_all(pool)
    .await
    .unwrap()
}

/// An upload that already exists, the way one the T3 upload route stored does:
/// a `User`-scoped row owned by whoever uploaded it. No S3 object behind it —
/// nothing in this file reads the bytes, only the row that names them.
async fn seed_upload(pool: &PgPool, owner: &str, filename: &str, mime_type: &str) -> String {
    seed_upload_sized(pool, owner, filename, mime_type, 2048).await
}

/// The same, at a size the caller cares about — what a member's ceiling is
/// measured against.
async fn seed_upload_sized(
    pool: &PgPool,
    owner: &str,
    filename: &str,
    mime_type: &str,
    size: i64,
) -> String {
    let file_id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO file_metadata (
            file_id, filename, mime_type, size, scope, owner_user_id, created_by, uploaded_at
         ) VALUES ($1, $2, $3, $4, 'user', $5, $5, NOW())",
    )
    .bind(&file_id)
    .bind(filename)
    .bind(mime_type)
    .bind(size)
    .bind(owner)
    .execute(pool)
    .await
    .unwrap();
    file_id
}

/// Whether the upload row is still there. `attachments.delete` and a re-POST
/// both remove it, so this is how a test sees that one of them did not.
async fn upload_exists(pool: &PgPool, file_id: &str) -> bool {
    sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM file_metadata WHERE file_id = $1)")
        .bind(file_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Whether the lifecycle was asked for a Sandbox. The record is the request,
/// not the outcome — the test runtime refuses to start anything.
async fn sandbox_requested(pool: &PgPool, task_id: &str) -> bool {
    sqlx::query_scalar::<_, i64>(
        r#"SELECT COUNT(*)
           FROM tasks t
           JOIN sandboxes s ON s.sandbox_id = COALESCE(t.sandbox_id, t.task_id)
           WHERE t.task_id = $1"#,
    )
    .bind(task_id)
    .fetch_one(pool)
    .await
    .unwrap()
        > 0
}

// =========================================================================
// Sending a turn
// =========================================================================

/// T3 mints the message id, renders an optimistic bubble under it, and retires
/// that bubble when the server's copy arrives carrying the same id. Minting our
/// own leaves two bubbles saying the same thing.
#[sqlx::test]
async fn a_turn_is_persisted_under_the_id_the_client_minted(pool: PgPool) {
    seed_project(&pool, USER_1).await;
    seed_task(&pool, "t-send", USER_1).await;

    sink(&pool)
        .send_turn(
            &viewer(USER_1),
            turn("t-send", "9f2b1e04-3c7a-4d51-8e66-0a1b2c3d4e5f", "hello"),
        )
        .await
        .expect("the turn is accepted");

    assert_eq!(
        message_uuids(&pool, "t-send").await,
        vec!["9f2b1e04-3c7a-4d51-8e66-0a1b2c3d4e5f".to_string()]
    );
}

/// A socket that drops mid-send is retried by the client with the same frame.
/// The unique index on `task_messages.uuid` is what makes that safe, and
/// reporting the rejection as success is what makes the retry terminate.
#[sqlx::test]
async fn the_same_frame_twice_is_one_message_and_two_successes(pool: PgPool) {
    seed_project(&pool, USER_1).await;
    seed_task(&pool, "t-replay", USER_1).await;

    let sink = sink(&pool);
    let uuid = "1a2b3c4d-5e6f-4071-8283-94a5b6c7d8e9";
    for attempt in 1..=2 {
        sink.send_turn(&viewer(USER_1), turn("t-replay", uuid, "hello"))
            .await
            .unwrap_or_else(|refusal| {
                panic!("attempt {attempt} was refused: {}", refusal.message())
            });
    }

    assert_eq!(
        message_uuids(&pool, "t-replay").await,
        vec![uuid.to_string()]
    );
}

/// A turn's images become uploads, and a deployment that cannot store an
/// upload cannot carry the turn either — this harness configures no S3, which
/// is the same condition as a deployment with `S3_BUCKET_NAME` unset.
///
/// The assertion that matters is the second one. Refusing is a message the
/// composer shows; sending the text without the image is a turn the person
/// believes carried their screenshot and an agent that never saw one.
#[sqlx::test]
async fn a_turn_whose_attachments_cannot_be_stored_is_refused_and_writes_nothing(pool: PgPool) {
    seed_project(&pool, USER_1).await;
    seed_task(&pool, "t-attach", USER_1).await;

    let mut turn = turn("t-attach", "2f6e5d4c-3b2a-4190-8877-665544332211", "look");
    turn.attachments = vec![moatless_t3code::ports::TurnAttachment::Inline {
        filename: "screenshot.png".into(),
        mime_type: "image/png".into(),
        bytes: vec![0u8; 16],
    }];

    let refusal = sink(&pool)
        .send_turn(&viewer(USER_1), turn)
        .await
        .expect_err("a turn whose image cannot be stored is refused");
    assert!(
        matches!(refusal, CommandRefusal::Failed { .. }),
        "storing failed, which is not the client asking for something impossible"
    );
    assert!(message_uuids(&pool, "t-attach").await.is_empty());
}

/// An attachment the client uploaded before it sent the turn arrives as an id.
/// It is already stored, so the send resolves it to the upload it names — no
/// second copy under a second id — and the Message ends up carrying exactly the
/// file the person attached.
#[sqlx::test]
async fn an_uploaded_attachment_is_resolved_rather_than_stored_again(pool: PgPool) {
    seed_project(&pool, USER_1).await;
    seed_task(&pool, "t-uploaded", USER_1).await;
    let attachment_id = seed_upload(&pool, USER_1, "report.pdf", "application/pdf").await;

    let mut turn = turn("t-uploaded", "3c7f6e5d-4b3a-42a1-8988-776655443322", "read this");
    turn.attachments = vec![moatless_t3code::ports::TurnAttachment::Uploaded {
        attachment_id: attachment_id.clone(),
        kind: AttachmentKind::File,
    }];

    sink(&pool)
        .send_turn(&viewer(USER_1), turn)
        .await
        .expect("an attachment that is already stored needs no S3");

    assert_eq!(
        message_file_ids(&pool, "t-uploaded").await,
        vec![attachment_id]
    );
}

/// The id reaches this server on a turn, and nothing about a turn proves the
/// person sending it minted that id. An upload somebody else owns, and an id
/// that names nothing, are the same refusal — a turn that could tell them apart
/// would be a way to ask whether an upload exists.
#[sqlx::test]
async fn a_turn_naming_an_attachment_its_sender_cannot_read_is_refused(pool: PgPool) {
    seed_project(&pool, USER_1).await;
    seed_task(&pool, "t-not-mine", USER_1).await;
    let theirs = seed_upload(&pool, USER_2, "theirs.pdf", "application/pdf").await;

    for attachment_id in [theirs, "6b1f7b2e-0000-4000-8000-00000000ffff".to_string()] {
        let mut turn = turn(
            "t-not-mine",
            "4d8a7b6c-5c4d-43b2-9a99-887766554433",
            "read this",
        );
        turn.attachments = vec![moatless_t3code::ports::TurnAttachment::Uploaded {
            attachment_id: attachment_id.clone(),
            kind: AttachmentKind::File,
        }];

        let refusal = sink(&pool)
            .send_turn(&viewer(USER_1), turn)
            .await
            .expect_err("an attachment this sender cannot read is refused");
        assert!(
            matches!(refusal, CommandRefusal::Rejected { .. }),
            "{attachment_id} produced {refusal:?}"
        );
    }
    assert!(message_uuids(&pool, "t-not-mine").await.is_empty());
}

/// An id on a turn is not proof it came from `attachments.createUploadUrl`.
/// Moatless's own multipart route stores files this viewer can read and knows
/// nothing about T3's union, so an upload past the member's ceiling — or under
/// the wrong member entirely — is refused rather than resolved. Otherwise the
/// caps would bind only the clients that already respect them, and the turn
/// would disagree with the projection, which drops the same attachment on the
/// way back out.
#[sqlx::test]
async fn a_turn_naming_an_upload_outside_its_member_is_refused(pool: PgPool) {
    seed_project(&pool, USER_1).await;
    seed_task(&pool, "t-oversize", USER_1).await;

    let too_big =
        seed_upload_sized(&pool, USER_1, "huge.bin", "application/octet-stream", 50 * 1024 * 1024 + 1)
            .await;
    let not_an_image = seed_upload(&pool, USER_1, "report.pdf", "application/pdf").await;
    let too_big_for_an_image =
        seed_upload_sized(&pool, USER_1, "huge.png", "image/png", 10 * 1024 * 1024 + 1).await;

    for (attachment_id, kind) in [
        (too_big, AttachmentKind::File),
        (not_an_image, AttachmentKind::Image),
        (too_big_for_an_image, AttachmentKind::Image),
    ] {
        let mut turn = turn(
            "t-oversize",
            "5e9b8a7c-6d5e-44c3-8b00-998877665544",
            "read this",
        );
        turn.attachments = vec![moatless_t3code::ports::TurnAttachment::Uploaded {
            attachment_id: attachment_id.clone(),
            kind,
        }];

        let refusal = sink(&pool)
            .send_turn(&viewer(USER_1), turn)
            .await
            .expect_err("an upload outside its member is refused");
        assert!(
            matches!(refusal, CommandRefusal::Rejected { .. }),
            "{attachment_id} produced {refusal:?}"
        );
    }
    assert!(message_uuids(&pool, "t-oversize").await.is_empty());
}

/// `attachments.delete` means "I abandoned this upload". Once a turn carries
/// the id, the upload is what the transcript, the download route and the
/// Sandbox pre-fetch all point at, so collecting it as an orphan would strip an
/// attachment off a message that was already sent and read. The composer
/// deletes whenever it clears a draft and cannot know which state the server is
/// in, so this is reported as success and the bytes are kept.
#[sqlx::test]
async fn an_attachment_a_message_carries_is_kept_when_the_composer_deletes_it(pool: PgPool) {
    seed_project(&pool, USER_1).await;
    seed_task(&pool, "t-keep", USER_1).await;
    let attachment_id = seed_upload(&pool, USER_1, "report.pdf", "application/pdf").await;

    let mut turn = turn("t-keep", "7a6b5c4d-8e9f-4012-8a3b-4c5d6e7f8091", "read this");
    turn.attachments = vec![moatless_t3code::ports::TurnAttachment::Uploaded {
        attachment_id: attachment_id.clone(),
        kind: AttachmentKind::File,
    }];
    sink(&pool)
        .send_turn(&viewer(USER_1), turn)
        .await
        .expect("the turn is accepted");

    let attachments = AppStateAttachments(test_app_state(pool.clone()));
    attachments
        .delete(&viewer(USER_1), &attachment_id)
        .await
        .expect("deleting a sent attachment is not an error the person can act on");

    assert!(
        upload_exists(&pool, &attachment_id).await,
        "the message still names this upload, so it is not an orphan to collect"
    );
}

/// The other half of the same rule: a re-`POST` of a still-valid upload URL
/// replaces the bytes, and a mint is good for ten minutes. Doing that after the
/// turn has been sent would change an attachment the agent has already read,
/// leaving the transcript describing bytes nobody can see. A genuine retry
/// cannot reach this — sending the turn required the first upload to finish.
#[sqlx::test]
async fn an_attachment_a_message_carries_cannot_be_replaced(pool: PgPool) {
    seed_project(&pool, USER_1).await;
    seed_task(&pool, "t-no-replace", USER_1).await;
    let attachment_id = seed_upload(&pool, USER_1, "report.pdf", "application/pdf").await;

    let mut turn = turn(
        "t-no-replace",
        "8b7c6d5e-9f01-4123-8b4c-5d6e7f809122",
        "read this",
    );
    turn.attachments = vec![moatless_t3code::ports::TurnAttachment::Uploaded {
        attachment_id: attachment_id.clone(),
        kind: AttachmentKind::File,
    }];
    sink(&pool)
        .send_turn(&viewer(USER_1), turn)
        .await
        .expect("the turn is accepted");

    let attachments = AppStateAttachments(test_app_state(pool.clone()));
    let refused = attachments
        .store(
            &viewer(USER_1),
            &attachment_id,
            moatless_t3code::ports::DeclaredAttachment {
                filename: "swapped.pdf".to_string(),
                mime_type: "application/pdf".to_string(),
            },
            vec![0u8; 16],
        )
        .await
        .expect_err("replacing a sent attachment is refused");
    assert!(matches!(
        refused,
        moatless_t3code::ports::AttachmentStoreError::Forbidden
    ));
    assert!(upload_exists(&pool, &attachment_id).await);
}

/// A message id arrives from the client, so it names a row the sender may have
/// no business touching. The id is unique across the whole table, not per Task,
/// and the write path it feeds upserts on that column — so a turn carrying
/// another conversation's message id must be refused before it reaches the
/// insert, or it rewrites that conversation's history from a Task the sender
/// does own.
#[sqlx::test]
async fn a_turn_reusing_another_conversations_message_id_is_refused(pool: PgPool) {
    seed_project(&pool, USER_1).await;
    seed_project_for(&pool, USER_2).await;
    seed_task(&pool, "t-mine", USER_1).await;
    seed_task_owned(&pool, "t-theirs", USER_2).await;

    let sink = sink(&pool);
    let uuid = "aaaa1111-2222-4333-8444-555566667777";
    sink.send_turn(&viewer(USER_2), turn("t-theirs", uuid, "their message"))
        .await
        .expect("the other conversation's own turn is accepted");

    let refusal = sink
        .send_turn(&viewer(USER_1), turn("t-mine", uuid, "overwritten"))
        .await
        .expect_err("a message id belonging to another conversation is refused");
    assert!(
        matches!(refusal, CommandRefusal::Rejected { .. }),
        "expected a rejection, got {refusal:?}"
    );

    assert_eq!(
        message_text(&pool, uuid).await.as_deref(),
        Some("their message"),
        "the other conversation's message was rewritten"
    );
    assert!(
        message_uuids(&pool, "t-mine").await.is_empty(),
        "the refused turn still wrote a row"
    );
}

/// The snapshot the client re-reads after sending is where the optimistic
/// bubble is reconciled. A message id that does not reach it is a bubble that
/// never retires.
#[sqlx::test]
async fn the_thread_snapshot_carries_the_message_id_that_was_sent(pool: PgPool) {
    seed_project(&pool, USER_1).await;
    seed_task(&pool, "t-snap", USER_1).await;

    let uuid = "44444444-5555-4666-8777-888899990000";
    sink(&pool)
        .send_turn(&viewer(USER_1), turn("t-snap", uuid, "hello"))
        .await
        .expect("the turn is accepted");

    let snapshot = thread_snapshot(
        &moatless_backend::t3code::conversations_over(&pool),
        &moatless_backend::t3code::conversations_over(&pool),
        &deployment(&defaults()),
        &viewer(USER_1),
        "t-snap",
    )
    .await
    .unwrap()
    .expect("the thread opens");

    let ids: Vec<&str> = snapshot.value["thread"]["messages"]
        .as_array()
        .expect("the snapshot carries a message list")
        .iter()
        .map(|message| message["id"].as_str().unwrap())
        .collect();
    assert!(
        ids.contains(&uuid),
        "the snapshot's message ids {ids:?} do not include the one the client sent"
    );
}

/// The refusal path a person meets, and the one that must not half-apply: a
/// closed Task takes no message, so nothing may be written before the check.
#[sqlx::test]
async fn a_send_to_a_closed_task_is_refused_and_writes_nothing(pool: PgPool) {
    seed_project(&pool, USER_1).await;
    let mut task = make_task_row("t-closed", USER_1);
    task.workspace_id = Some(WS.to_string());
    task.status = TaskStatus::Closed;
    moatless_backend::task::dao::store_over(&pool).insert_task(&task).await.unwrap();

    let refusal = sink(&pool)
        .send_turn(
            &viewer(USER_1),
            turn("t-closed", "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee", "hello"),
        )
        .await
        .expect_err("a closed task refuses");

    assert!(
        matches!(refusal, CommandRefusal::Rejected { .. }),
        "a closed task is a rejection, not a failure: {refusal:?}"
    );
    assert!(message_uuids(&pool, "t-closed").await.is_empty());
}

/// Access is decided on this side of the wire. The listing already hides what a
/// viewer may not read; the write path has to refuse it too, or a guessed id is
/// a way into someone else's conversation.
#[sqlx::test]
async fn a_send_by_a_viewer_who_cannot_read_the_task_is_refused(pool: PgPool) {
    seed_project(&pool, USER_1).await;
    seed_task(&pool, "t-private", USER_1).await;

    let refusal = sink(&pool)
        .send_turn(
            &viewer(USER_2),
            turn("t-private", "bbbbbbbb-cccc-4ddd-8eee-ffffffffffff", "hello"),
        )
        .await
        .expect_err("a task this viewer cannot read refuses");

    assert!(
        matches!(refusal, CommandRefusal::NotWritable { .. }),
        "an unreadable task is indistinguishable from a missing one: {refusal:?}"
    );
    assert!(message_uuids(&pool, "t-private").await.is_empty());
}

/// The model reaches the row the turn is stored in, which is what the agent
/// runs on and what the sidebar reads back.
#[sqlx::test]
async fn the_selected_model_is_persisted_with_the_turn(pool: PgPool) {
    seed_project(&pool, USER_1).await;
    seed_task(&pool, "t-model", USER_1).await;

    let mut requested = turn("t-model", "cccccccc-dddd-4eee-8fff-000011112222", "hello");
    requested.model = Some("sonnet".into());
    requested.agent_mode = Some(AgentMode("plan".into()));
    sink(&pool)
        .send_turn(&viewer(USER_1), requested)
        .await
        .expect("the turn is accepted");

    let model: Option<String> =
        sqlx::query_scalar("SELECT model FROM task_messages WHERE uuid = $1")
            .bind("cccccccc-dddd-4eee-8fff-000011112222")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(model.as_deref(), Some("sonnet"));
}

// =========================================================================
// Creating a thread
// =========================================================================

/// T3 navigates to the Thread id before the command is answered, so a Task
/// created under any other id is one the client is already looking at the wrong
/// side of.
#[sqlx::test]
async fn a_bootstrapped_thread_is_created_under_the_clients_id_with_its_message(pool: PgPool) {
    seed_project(&pool, USER_1).await;

    let task_id = "77777777-8888-4999-8aaa-bbbbccccdddd";
    let message_id = "eeeeeeee-ffff-4000-8111-222233334444";
    sink(&pool)
        .create_thread(
            &viewer(USER_1),
            NewThread {
                task_id: task_id.into(),
                workspace_id: WS.into(),
                instance: "claudeAgent".into(),
                branch: None,
                model: None,
                first_message: Some(turn(task_id, message_id, "get started")),
            },
        )
        .await
        .expect("the thread is created");

    let task = moatless_backend::task::dao::store_over(&pool)
        .find_by_id(task_id, &viewer(USER_1))
        .await
        .unwrap()
        .expect("the task exists under the id T3 minted");
    assert_eq!(
        task.name, None,
        "the client's title is its own; the agent names the Task on its first turn"
    );
    assert_eq!(task.agent_type, AgentType(catalog::CLAUDE_CODE.id.into()));

    assert_eq!(
        message_uuids(&pool, task_id).await,
        vec![message_id.to_string()],
        "the first message is stored under the id T3 minted for it"
    );
    assert!(
        sandbox_requested(&pool, task_id).await,
        "a thread with something to say needs a sandbox to say it to"
    );
}

/// The other half of Decision 5: a Thread created with nothing in it is a Task
/// nobody has spoken to. Provisioning it burns a Sandbox on a draft that may
/// never be used.
#[sqlx::test]
async fn a_thread_created_with_no_message_gets_no_sandbox(pool: PgPool) {
    seed_project(&pool, USER_1).await;

    let task_id = "12121212-3434-4565-8787-909090909090";
    sink(&pool)
        .create_thread(
            &viewer(USER_1),
            NewThread {
                task_id: task_id.into(),
                workspace_id: WS.into(),
                instance: "claudeAgent".into(),
                branch: None,
                model: None,
                first_message: None,
            },
        )
        .await
        .expect("the thread is created");

    moatless_backend::task::dao::store_over(&pool)
        .find_by_id(task_id, &viewer(USER_1))
        .await
        .unwrap()
        .expect("the task exists");

    assert!(message_uuids(&pool, task_id).await.is_empty());
    assert!(
        !sandbox_requested(&pool, task_id).await,
        "an empty thread was provisioned"
    );
}

/// The Thread stays writable after being created empty — this is the sequence a
/// T3 draft actually takes, and the point where the Sandbox is finally asked
/// for.
#[sqlx::test]
async fn sending_into_an_unprovisioned_thread_provisions_it(pool: PgPool) {
    seed_project(&pool, USER_1).await;

    let task_id = "56565656-7878-4909-8121-343434343434";
    let sink = sink(&pool);
    sink.create_thread(
        &viewer(USER_1),
        NewThread {
            task_id: task_id.into(),
            workspace_id: WS.into(),
            instance: "claudeAgent".into(),
            branch: None,
            model: None,
            first_message: None,
        },
    )
    .await
    .expect("the thread is created");
    assert!(!sandbox_requested(&pool, task_id).await);

    sink.send_turn(
        &viewer(USER_1),
        turn(task_id, "98989898-7676-4545-8323-101010101010", "now start"),
    )
    .await
    .expect("the turn is accepted");

    assert_eq!(message_uuids(&pool, task_id).await.len(), 1);
    assert!(sandbox_requested(&pool, task_id).await);
}

// =========================================================================
// Forking a thread
// =========================================================================

/// The row `task_forks` keeps for `task_id`, if it is a fork of anything.
async fn fork_row(pool: &PgPool, task_id: &str) -> Option<(String, Option<i32>)> {
    sqlx::query_as(
        r#"SELECT source_task_id, at_turn FROM task_forks WHERE task_id = $1"#,
    )
    .bind(task_id)
    .fetch_optional(pool)
    .await
    .unwrap()
}

fn fork(new_task_id: &str, source_task_id: &str) -> ForkThread {
    ForkThread {
        new_task_id: new_task_id.into(),
        source_task_id: source_task_id.into(),
        at_turn: None,
        message: None,
        same_sandbox: false,
        model: None,
        agent_mode: None,
        effort: None,
    }
}

/// T3 navigates to the fork's id before the command is answered, same as
/// `thread.create` — so the Task has to exist under exactly that id, carrying
/// which Task it branched from.
#[sqlx::test]
async fn a_fork_is_created_under_the_clients_id_referencing_its_source(pool: PgPool) {
    seed_project(&pool, USER_1).await;
    seed_task(&pool, "t-source", USER_1).await;

    let fork_id = "44444444-5555-4666-8777-888899990000";
    sink(&pool)
        .fork(&viewer(USER_1), fork(fork_id, "t-source"))
        .await
        .expect("the fork is created");

    let task = moatless_backend::task::dao::store_over(&pool)
        .find_by_id(fork_id, &viewer(USER_1))
        .await
        .unwrap()
        .expect("the fork exists under the id T3 minted");
    assert_eq!(task.agent_type, AgentType(catalog::CLAUDE_CODE.id.into()));

    let (source_task_id, at_turn) = fork_row(&pool, fork_id)
        .await
        .expect("the fork's task_forks row names its source");
    assert_eq!(source_task_id, "t-source");
    assert_eq!(at_turn, None);
}

/// A viewer who cannot read the source Task gets the same answer as one that
/// does not exist — existence hiding, as everywhere else this surface refuses.
#[sqlx::test]
async fn a_fork_of_a_thread_the_viewer_cannot_read_is_not_writable(pool: PgPool) {
    seed_project_for(&pool, USER_2).await;
    seed_task_owned(&pool, "t-other", USER_2).await;

    let fork_id = "99998888-7777-4666-8555-444433332222";
    let error = sink(&pool)
        .fork(&viewer(USER_1), fork(fork_id, "t-other"))
        .await
        .expect_err("the source thread is not this viewer's to fork");
    assert!(matches!(error, CommandRefusal::NotWritable { .. }));
}

// =========================================================================
// Renaming
// =========================================================================

/// The one thing T3 changes about a Thread that is not a message. A rename by
/// somebody who cannot read it is refused for the same reason a send is.
#[sqlx::test]
async fn a_rename_reaches_the_task_and_stops_at_the_access_rule(pool: PgPool) {
    seed_project(&pool, USER_1).await;
    seed_task(&pool, "t-name", USER_1).await;

    let sink = sink(&pool);
    sink.update_meta(
        &viewer(USER_1),
        ThreadMeta {
            task_id: "t-name".into(),
            title: Some("Renamed from T3".into()),
            model: None,
        },
    )
    .await
    .expect("the rename is accepted");

    let name: Option<String> = sqlx::query_scalar("SELECT name FROM tasks WHERE task_id = $1")
        .bind("t-name")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(name.as_deref(), Some("Renamed from T3"));

    let refusal = sink
        .update_meta(
            &viewer(USER_2),
            ThreadMeta {
                task_id: "t-name".into(),
                title: Some("Renamed by a stranger".into()),
                model: None,
            },
        )
        .await
        .expect_err("a viewer who cannot read the task cannot rename it");
    assert!(
        matches!(refusal, CommandRefusal::NotWritable { .. }),
        "a rename that changed nothing must not report success: {refusal:?}"
    );

    let unchanged: Option<String> = sqlx::query_scalar("SELECT name FROM tasks WHERE task_id = $1")
        .bind("t-name")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(unchanged.as_deref(), Some("Renamed from T3"));
}

/// A meta update carrying neither field is a no-op rather than an empty patch —
/// T3 sends one on every model change it decides not to apply.
#[sqlx::test]
async fn an_empty_meta_update_changes_nothing(pool: PgPool) {
    seed_project(&pool, USER_1).await;
    seed_task(&pool, "t-noop", USER_1).await;

    let before: Option<String> = sqlx::query_scalar("SELECT name FROM tasks WHERE task_id = $1")
        .bind("t-noop")
        .fetch_one(&pool)
        .await
        .unwrap();

    sink(&pool)
        .update_meta(
            &viewer(USER_1),
            ThreadMeta {
                task_id: "t-noop".into(),
                title: None,
                model: None,
            },
        )
        .await
        .expect("an empty update is accepted");

    let after: Option<String> = sqlx::query_scalar("SELECT name FROM tasks WHERE task_id = $1")
        .bind("t-noop")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(before, after);
}

// =========================================================================
// Settle and snooze
// =========================================================================

#[sqlx::test]
async fn settling_as_one_follower_leaves_another_followers_row_untouched(pool: PgPool) {
    seed_project(&pool, USER_1).await;
    seed_public_task(&pool, "t-settle-one", USER_1).await;
    follow(&pool, USER_1, "t-settle-one").await;
    follow(&pool, USER_2, "t-settle-one").await;

    sink(&pool)
        .settle(&viewer(USER_1), "t-settle-one")
        .await
        .expect("settle is accepted");

    assert_eq!(
        follow_state(&pool, USER_1, "t-settle-one")
            .await
            .0
            .as_deref(),
        Some("settled")
    );
    assert_eq!(
        follow_state(&pool, USER_2, "t-settle-one").await,
        (None, None, None, None)
    );
}

#[sqlx::test]
async fn settling_stops_the_sandbox(pool: PgPool) {
    seed_project(&pool, USER_1).await;
    seed_task(&pool, "t-settle-stops", USER_1).await;
    follow(&pool, USER_1, "t-settle-stops").await;

    sink(&pool)
        .settle(&viewer(USER_1), "t-settle-stops")
        .await
        .expect("settle is accepted");

    assert_eq!(
        sandbox_intent(&pool, "t-settle-stops").await,
        Some(("stopped".to_string(), Some("user".to_string())))
    );
}

/// The settlement is the durable part. A follower who may not control the Task
/// has no Sandbox to stop, and that is not a reason to report the settle — which
/// already landed — as refused.
#[sqlx::test]
async fn settling_a_thread_the_follower_cannot_control_still_settles(pool: PgPool) {
    seed_project(&pool, USER_1).await;
    seed_public_task(&pool, "t-settle-unowned", USER_1).await;
    follow(&pool, USER_2, "t-settle-unowned").await;

    sink(&pool)
        .settle(&viewer(USER_2), "t-settle-unowned")
        .await
        .expect("settle is accepted");

    assert_eq!(
        follow_state(&pool, USER_2, "t-settle-unowned")
            .await
            .0
            .as_deref(),
        Some("settled")
    );
    assert_eq!(sandbox_intent(&pool, "t-settle-unowned").await, None);
}

#[sqlx::test]
async fn settling_twice_keeps_the_first_timestamp_and_records_one_change(pool: PgPool) {
    seed_project(&pool, USER_1).await;
    seed_task(&pool, "t-settle-idempotent", USER_1).await;
    follow(&pool, USER_1, "t-settle-idempotent").await;
    let before = listing_change_count(&pool, "t-settle-idempotent").await;

    let sink = sink(&pool);
    sink.settle(&viewer(USER_1), "t-settle-idempotent")
        .await
        .expect("first settle is accepted");
    let first = follow_state(&pool, USER_1, "t-settle-idempotent")
        .await
        .1
        .expect("first settle timestamp");
    // Settling also releases the sandbox it was holding, and that intent move
    // is a listing change of its own — a different axis, announced separately.
    // So what the repeat must add is nothing, measured against where the first
    // settle left the counter rather than against a fixed number of changes.
    let after_first = listing_change_count(&pool, "t-settle-idempotent").await;
    assert!(
        after_first > before,
        "the first settle is announced to the listing"
    );

    sink.settle(&viewer(USER_1), "t-settle-idempotent")
        .await
        .expect("second settle is accepted");

    let second = follow_state(&pool, USER_1, "t-settle-idempotent")
        .await
        .1
        .expect("second settle timestamp");
    assert_eq!(first, second);
    assert_eq!(
        listing_change_count(&pool, "t-settle-idempotent").await,
        after_first,
        "a repeated settle changes nothing, so it announces nothing"
    );
}

#[sqlx::test]
async fn snoozing_twice_to_the_same_wake_time_keeps_the_first_timestamp(pool: PgPool) {
    seed_project(&pool, USER_1).await;
    seed_task(&pool, "t-snooze-idempotent", USER_1).await;
    follow(&pool, USER_1, "t-snooze-idempotent").await;
    let until = Utc::now() + Duration::hours(1);

    let sink = sink(&pool);
    sink.snooze(&viewer(USER_1), "t-snooze-idempotent", until)
        .await
        .expect("first snooze is accepted");
    let first = follow_state(&pool, USER_1, "t-snooze-idempotent")
        .await
        .3
        .expect("first snooze timestamp");
    sink.snooze(&viewer(USER_1), "t-snooze-idempotent", until)
        .await
        .expect("second snooze is accepted");

    let second = follow_state(&pool, USER_1, "t-snooze-idempotent")
        .await
        .3
        .expect("second snooze timestamp");
    assert_eq!(first, second);
}

#[sqlx::test]
async fn snoozing_to_a_different_wake_time_stamps_fresh(pool: PgPool) {
    seed_project(&pool, USER_1).await;
    seed_task(&pool, "t-snooze-fresh", USER_1).await;
    follow(&pool, USER_1, "t-snooze-fresh").await;

    let sink = sink(&pool);
    sink.snooze(
        &viewer(USER_1),
        "t-snooze-fresh",
        Utc::now() + Duration::hours(1),
    )
    .await
    .expect("first snooze is accepted");
    let first = follow_state(&pool, USER_1, "t-snooze-fresh")
        .await
        .3
        .expect("first snooze timestamp");
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    sink.snooze(
        &viewer(USER_1),
        "t-snooze-fresh",
        Utc::now() + Duration::hours(2),
    )
    .await
    .expect("second snooze is accepted");

    let second = follow_state(&pool, USER_1, "t-snooze-fresh")
        .await
        .3
        .expect("second snooze timestamp");
    assert!(second > first);
}

#[sqlx::test]
async fn unsettle_writes_active_rather_than_null(pool: PgPool) {
    seed_project(&pool, USER_1).await;
    seed_task(&pool, "t-unsettle", USER_1).await;
    follow(&pool, USER_1, "t-unsettle").await;

    let sink = sink(&pool);
    sink.settle(&viewer(USER_1), "t-unsettle")
        .await
        .expect("settle is accepted");
    sink.unsettle(&viewer(USER_1), "t-unsettle")
        .await
        .expect("unsettle is accepted");

    let state = follow_state(&pool, USER_1, "t-unsettle").await;
    assert_eq!(state.0.as_deref(), Some("active"));
    assert!(state.1.is_none());
}

#[sqlx::test]
async fn settle_on_a_task_the_viewer_cannot_read_is_not_writable(pool: PgPool) {
    seed_project(&pool, USER_1).await;
    seed_task(&pool, "t-settle-private", USER_1).await;
    follow(&pool, USER_1, "t-settle-private").await;

    let refusal = sink(&pool)
        .settle(&viewer(USER_2), "t-settle-private")
        .await
        .expect_err("unreadable task refuses");
    assert!(matches!(refusal, CommandRefusal::NotWritable { .. }));
}

/// Every method on the port is `Send + Sync` behind an `Arc`, because the
/// socket holds it across await points on several concurrent streams. A method
/// that is not compiles here and nowhere else.
#[allow(dead_code)]
fn the_sink_is_usable_from_the_socket(pool: PgPool) -> Arc<dyn ThreadCommandSink> {
    Arc::new(AppStateCommands(test_app_state(pool)))
}
