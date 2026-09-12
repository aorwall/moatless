//! The rest of the change kinds, delivered.
//!
//! Ticket 04 delivered a turn moving. What follows is everything else a sidebar
//! shows moving, and the two directions ticket 04 could not express: a row
//! *entering* a listing, and a row *leaving* one.

use super::feed::{Held, delivered, one_item, one_row, recorded_after, watcher, watching};
use super::harness::{head, project, reads, thread_shell_row, viewer};
use super::seeding::{
    follow, seed_agent_message, seed_live_project, seed_task, seed_user_message, seed_workspaces,
    unfollow,
};
use crate::common::USER_1;
use crate::common::authed_request;
use crate::common::make_message_row;
use crate::common::make_repository_row;
use crate::common::make_task_row;
use crate::common::make_workspace_row;
use crate::common::run_config;
use crate::common::seed_team;
use crate::common::test_app_state;
use axum::http::StatusCode;
use futures::future::BoxFuture;
use moatless_tasks::message::models::MessageType;
use moatless_tasks::message::models::TaskMessage;
use moatless_tasks::tag::test_support as tag_fixtures;
use moatless_tasks::task::models::TaskPatch;
use moatless_tasks::task::models::TaskStatus;
use moatless_tasks::task::models::Visibility;
use moatless_tasks::task_activities::mapping::BlockContext;
use moatless_tasks::task_activities::mapping::ConversationWrite;
use moatless_tasks::task_activities::mapping::map_content;
use moatless_types::access_control::Scope;
use moatless_types::access_control::ScopeContext;
use moatless_workspaces::compose;
use moatless_workspaces::repository::routes::repository_router;
use moatless_workspaces::workspace::models::WorkspacePatch;
use serde_json::Value;
use serde_json::json;
use sqlx::PgPool;
use tower::ServiceExt;

/// The one change kind that alters a sidebar's *membership* rather than the
/// contents of a row already in it, so a client that handles every other kind
/// can still miss this one entirely.
#[sqlx::test]
async fn following_a_conversation_makes_it_appear_and_unfollowing_removes_it(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-live", "user-1", Some("ws-live")).await;
    // Readable but not followed, so it is absent from the opening listing —
    // which is what makes the appearance below a change of membership.
    unfollow(&pool, "user-1", "t-live").await;

    let viewer = viewer("user-1");
    let (mut feed, mut held) = watcher(&pool, &viewer).await;
    assert!(held.threads.is_empty(), "nothing is followed yet");

    follow(&pool, "user-1", "t-live").await;
    held.poll(&pool, &mut feed, &viewer).await;
    assert_eq!(
        held.threads,
        ["t-live"],
        "following puts the row on the screen"
    );

    unfollow(&pool, "user-1", "t-live").await;
    let item = one_item(&pool, &mut feed, &viewer).await;
    assert_eq!(item["kind"], "thread-removed");
    assert_eq!(item["threadId"], "t-live");
    held.apply(&item);
    assert!(held.threads.is_empty(), "unfollowing takes it off again");
}

/// The disclosure rule, asserted rather than described.
///
/// A conversation leaves someone's listing, and every other watching client
/// re-reads it and finds nothing — indistinguishable, from the re-read alone,
/// from a conversation they never had. T3's own server emits `thread-removed`
/// on that empty re-read (`apps/server/src/ws.ts`), which on a shared
/// deployment hands every connected client the id of every conversation that
/// was unfollowed, made private or deleted anywhere.
///
/// The watcher here holds a conversation of their own, so an implementation
/// that emitted removals only when it held *nothing* would still fail this.
#[sqlx::test]
async fn a_removal_names_only_a_conversation_this_client_was_sent(pool: PgPool) {
    seed_live_project(&pool).await;
    // `user-2` follows one conversation and can read neither of `user-1`'s.
    seed_task(&pool, "t-theirs", "user-1", Some("ws-live")).await;
    seed_task(&pool, "t-deleted", "user-1", Some("ws-live")).await;
    let mut mine = make_task_row("t-mine", "user-2");
    mine.workspace_id = Some("ws-live".to_string());
    mine.visibility = Visibility::Public;
    moatless_tasks::task::test_support::seed_task(&pool, &mine)
        .await
        .unwrap();

    let watcher_viewer = viewer("user-2");
    let (mut feed, mut held) = watcher(&pool, &watcher_viewer).await;
    assert_eq!(
        held.threads,
        ["t-mine"],
        "they hold their own and nothing else"
    );
    let watched_from = reads(&pool).listing_head().await.unwrap().position;

    // Two ways a row leaves someone else's listing, neither of them this
    // viewer's business.
    unfollow(&pool, "user-1", "t-theirs").await;
    assert!(
        moatless_tasks::task::test_support::remove_task(&pool, "t-deleted", &viewer("user-1"))
            .await
            .unwrap()
    );
    // The premise: the changes really were recorded, so silence below is the
    // feed filtering rather than nothing having happened.
    assert_eq!(recorded_after(&pool, watched_from).await, 2);

    assert_eq!(
        delivered(&pool, &mut feed, &watcher_viewer).await,
        Vec::<Value>::new(),
        "a client was told the id of a conversation it was never sent"
    );
    assert_eq!(held.threads, ["t-mine"]);
    held.poll(&pool, &mut feed, &watcher_viewer).await;
}

#[sqlx::test]
async fn deleting_a_conversation_removes_it_from_a_watching_client(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-live", "user-1", Some("ws-live")).await;

    let viewer = viewer("user-1");
    let (mut feed, mut held) = watcher(&pool, &viewer).await;
    assert_eq!(held.threads, ["t-live"]);

    assert!(
        moatless_tasks::task::test_support::remove_task(&pool, "t-live", &viewer)
            .await
            .unwrap()
    );

    let item = one_item(&pool, &mut feed, &viewer).await;
    assert_eq!(item["kind"], "thread-removed");
    assert_eq!(item["threadId"], "t-live");
    held.apply(&item);
    assert!(held.threads.is_empty());
}

/// Closing is the way a row leaves that a person performs on purpose, and the
/// one that comes back: the conversation is not deleted and the follow survives
/// it, so reopening puts the row on the screen again.
///
/// A watching client is told, rather than left holding a conversation the next
/// full listing would not contain — the drift that shows as a sidebar row that
/// only disappears on reload.
///
/// It is told with the row rather than with a removal, because the conversation
/// has not left the viewer's world: they still follow it and may still read it,
/// and `archivedAt` is what puts it under the archive instead of the sidebar. A
/// removal would say something stronger and, on a resumed feed, would not be
/// sent at all.
#[sqlx::test]
async fn closing_a_conversation_removes_it_from_a_watching_client(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-live", "user-1", Some("ws-live")).await;

    let viewer = viewer("user-1");
    let (mut feed, mut held) = watcher(&pool, &viewer).await;
    assert_eq!(held.threads, ["t-live"]);

    assert!(
        moatless_tasks::task::test_support::set_status_if_open(
            &pool,
            "t-live",
            TaskStatus::Closed,
            &viewer
        )
        .await
        .unwrap()
    );

    let item = one_row(&pool, &mut feed, &viewer).await;
    assert_eq!(item["thread"]["id"], "t-live");
    assert!(!item["thread"]["archivedAt"].is_null());
    held.apply(&item);
    assert!(held.threads.is_empty());

    moatless_tasks::task::test_support::patch_task(
        &pool,
        "t-live",
        &TaskPatch {
            status: Some(TaskStatus::Open),
            ..TaskPatch::default()
        },
        &viewer,
    )
    .await
    .unwrap();

    let item = one_row(&pool, &mut feed, &viewer).await;
    assert_eq!(
        item["thread"]["id"], "t-live",
        "reopening puts the row back"
    );
    held.apply(&item);
    assert_eq!(held.threads, ["t-live"]);
}

/// Losing read access is the fourth way a row leaves, and the one no mutation of
/// the conversation's own row expresses: the follow survives, the conversation
/// survives, and it still has to go.
#[sqlx::test]
async fn a_conversation_that_stops_being_readable_is_removed(pool: PgPool) {
    seed_live_project(&pool).await;
    let mut shared = make_task_row("t-shared", "user-1");
    shared.workspace_id = Some("ws-live".to_string());
    shared.visibility = Visibility::Public;
    moatless_tasks::task::test_support::seed_task(&pool, &shared)
        .await
        .unwrap();
    follow(&pool, "user-2", "t-shared").await;

    let watcher_viewer = viewer("user-2");
    let (mut feed, mut held) = watcher(&pool, &watcher_viewer).await;
    assert_eq!(held.threads, ["t-shared"]);

    moatless_tasks::task::test_support::patch_task(
        &pool,
        "t-shared",
        &TaskPatch {
            visibility: Some(Visibility::Private),
            ..TaskPatch::default()
        },
        &viewer("user-1"),
    )
    .await
    .unwrap();

    let item = one_item(&pool, &mut feed, &watcher_viewer).await;
    assert_eq!(item["kind"], "thread-removed");
    assert_eq!(item["threadId"], "t-shared");
    held.apply(&item);
    assert!(held.threads.is_empty());
}

/// Every remaining conversation change kind, each one asserted to move the row
/// a watching client holds — and to move the *value* that kind changes, so a
/// site that recorded a change without writing anything would not pass.
#[sqlx::test]
async fn every_remaining_change_kind_moves_the_row(pool: PgPool) {
    seed_live_project(&pool).await;
    sqlx::query("INSERT INTO tags (tag_id, name) VALUES ('tag-1', 'urgent')")
        .execute(&pool)
        .await
        .unwrap();

    let viewer = viewer("user-1");
    let owner = viewer.clone();

    // Each case: what a person did, and what the delivered row must say
    // afterwards. `None` asserts only that the row was delivered — the tag set
    // and the expiry are not values the shell row carries, and a change to them
    // still has to move the conversation in the sidebar.
    #[allow(clippy::type_complexity)]
    let cases: &[(
        &str,
        fn(PgPool, ScopeContext, String) -> BoxFuture<'static, ()>,
        Option<(&str, Value)>,
    )] = &[
        (
            "renamed",
            |pool, ctx, id| {
                Box::pin(async move {
                    let patch = TaskPatch {
                        name: Some("a new name".into()),
                        ..TaskPatch::default()
                    };
                    moatless_tasks::task::test_support::patch_task(&pool, &id, &patch, &ctx)
                        .await
                        .unwrap();
                })
            },
            Some(("title", json!("a new name"))),
        ),
        (
            "described",
            |pool, ctx, id| {
                Box::pin(async move {
                    let patch = TaskPatch {
                        description: Some("a description".into()),
                        ..TaskPatch::default()
                    };
                    moatless_tasks::task::test_support::patch_task(&pool, &id, &patch, &ctx)
                        .await
                        .unwrap();
                })
            },
            None,
        ),
        // Closing and reopening are deliberately not here: they change the
        // sidebar's *membership* rather than a row in it, and are asserted as
        // such in `closing_a_conversation_removes_it_from_a_watching_client`.
        (
            "given a new expiry",
            |pool, ctx, id| {
                Box::pin(async move {
                    let patch = TaskPatch {
                        ttl_minutes: Some(240),
                        ..TaskPatch::default()
                    };
                    moatless_tasks::task::test_support::patch_task(&pool, &id, &patch, &ctx)
                        .await
                        .unwrap();
                })
            },
            None,
        ),
        (
            "tagged",
            |pool, _ctx, id| {
                Box::pin(async move {
                    tag_fixtures::add_task_tags(&pool, &id, &["tag-1".to_string()])
                        .await
                        .unwrap();
                })
            },
            None,
        ),
        (
            "given a new tag set",
            |pool, _ctx, id| {
                Box::pin(async move {
                    // Replacing nothing with nothing moves no sidebar and is
                    // deliberately not recorded, so this case has to start from
                    // a tag there is something to replace.
                    tag_fixtures::add_task_tags(&pool, &id, &["tag-1".to_string()])
                        .await
                        .unwrap();
                    tag_fixtures::set_task_tags(&pool, &id, &[]).await.unwrap();
                })
            },
            None,
        ),
        (
            "written to by a person",
            |pool, _ctx, id| {
                Box::pin(async move {
                    let msg = make_message_row(&id, &format!("{id}-said"));
                    moatless_tasks::message::test_support::seed_message(
                        &pool,
                        &msg,
                        &[],
                        &ConversationWrite::default(),
                    )
                    .await
                    .unwrap();
                })
            },
            // The activity timestamp T3 orders the sidebar by. It was null on
            // a conversation nobody had written to yet.
            Some(("hasPendingUserInput", json!(false))),
        ),
        (
            "asked a question",
            |pool, _ctx, id| {
                Box::pin(async move {
                    let uuid = format!("{id}-asked");
                    let msg = TaskMessage {
                        message_type: MessageType::Assistant,
                        message_text: Some("Which one?".into()),
                        ..make_message_row(&id, &uuid)
                    };
                    let question = map_content(
                        BlockContext {
                            task_id: &id,
                            message_uuid: &uuid,
                            parent_uuid: None,
                            message_type: MessageType::Assistant,
                            block_index: 1,
                        },
                        &json!([{
                            "type": "tool_use",
                            "id": "tu-1",
                            "name": "AskUserQuestion",
                            "input": {}
                        }]),
                    );
                    moatless_tasks::message::test_support::seed_message(
                        &pool,
                        &msg,
                        &[],
                        &question,
                    )
                    .await
                    .unwrap();
                })
            },
            Some(("hasPendingUserInput", json!(true))),
        ),
    ];

    for (index, (what, run, expected)) in cases.iter().enumerate() {
        let task_id = format!("t-{index}");
        seed_task(&pool, &task_id, "user-1", Some("ws-live")).await;

        // Watch from after the seeding, so the only change the feed can carry
        // is the one this case makes.
        let mut feed = watching(&pool, &viewer).await;
        let watched_from = reads(&pool).listing_head().await.unwrap().position;
        run(pool.clone(), owner.clone(), task_id.clone()).await;
        assert!(
            recorded_after(&pool, watched_from).await > 0,
            "a conversation `{what}` recorded no listing change, \
             so nothing about it reaches a sidebar"
        );

        let event = one_row(&pool, &mut feed, &viewer).await;
        assert_eq!(
            event["thread"]["id"], task_id,
            "a conversation `{what}` delivered someone else's row"
        );
        if let Some((key, value)) = expected {
            assert_eq!(
                event["thread"][key], *value,
                "a conversation `{what}` delivered a row whose `{key}` had not moved"
            );
        }
    }
}

/// The acceptance criterion the message-send path exists to satisfy, in the
/// order `message::handlers` performs it: follow first, write second.
///
/// The two changes land between the same pair of polls and coalesce into one
/// re-read, so what the client gets is the row *and* the message, never the
/// message against a row it does not hold.
#[sqlx::test]
async fn writing_to_a_conversation_the_author_did_not_follow_adds_the_row(pool: PgPool) {
    // Deployment-scoped, so `user-2` may read the project as well as the
    // conversation — the case where a delta is enough.
    let mut shared_project = make_workspace_row("ws-shared", "user-1");
    shared_project.scope = Scope::Global;
    shared_project.owner_user_id = None;
    shared_project.created_by = None;
    compose::workspaces(pool.clone())
        .insert_workspace(&shared_project)
        .await
        .unwrap();
    let mut theirs = make_task_row("t-theirs", "user-1");
    theirs.workspace_id = Some("ws-shared".to_string());
    theirs.visibility = Visibility::Public;
    moatless_tasks::task::test_support::seed_task(&pool, &theirs)
        .await
        .unwrap();

    let author = viewer("user-2");
    let (mut feed, mut held) = watcher(&pool, &author).await;
    assert!(held.threads.is_empty(), "they do not follow it yet");
    assert_eq!(held.projects, ["ws-shared"]);

    // The order `send_message` writes in.
    follow(&pool, "user-2", "t-theirs").await;
    seed_user_message(&pool, "t-theirs", "m-1").await;

    let items = held.poll(&pool, &mut feed, &author).await;
    assert_eq!(items.len(), 1, "one row, carrying both changes: {items:#?}");
    assert_eq!(items[0]["kind"], "thread-upserted");
    assert_eq!(held.threads, ["t-theirs"]);
    assert_eq!(
        items[0]["thread"]["projectId"], "ws-shared",
        "the row arrived under a project the client is holding, or it is dropped"
    );
    assert!(
        !items[0]["thread"]["latestUserMessageAt"].is_null(),
        "the message the author just wrote is on the row"
    );
}

/// The same criterion where a delta cannot express it: the conversation is
/// readable and its project is not, so the client has never been sent the
/// project the row needs to hang under.
///
/// Two events at one position deliver one — the client advances its position on
/// the first and discards the second (`shellReducer.ts:15`) — and there is no
/// position below the conversation's to put the project at, because the
/// conversation's change may sit immediately after the last one delivered. So
/// the feed sends the listing whole instead, which the contract declares as a
/// stream item and the client applies unconditionally.
///
/// Against the obvious implementation — emit the `thread-upserted` alone — this
/// test fails on the project assertion, and a real client would drop the row
/// with nothing logged.
#[sqlx::test]
async fn a_conversation_entering_a_project_the_client_lacks_arrives_as_a_listing(pool: PgPool) {
    seed_workspaces(&pool).await;
    let mut theirs = make_task_row("t-theirs", "user-2");
    // `ws-other` belongs to `user-2` and `user-1` cannot read it.
    theirs.workspace_id = Some("ws-other".to_string());
    theirs.visibility = Visibility::Public;
    // `tasks_owner_scope_chk`: a deployment-scoped row has no owner.
    theirs.scope = Scope::Global;
    theirs.owner_user_id = None;
    moatless_tasks::task::test_support::seed_task(&pool, &theirs)
        .await
        .unwrap();

    let watcher_viewer = viewer("user-1");
    let (mut feed, mut held) = watcher(&pool, &watcher_viewer).await;
    assert!(held.threads.is_empty());
    assert!(
        !held.projects.contains(&"ws-other".to_string()),
        "the private project is not in their opening listing"
    );

    follow(&pool, "user-1", "t-theirs").await;

    let item = one_item(&pool, &mut feed, &watcher_viewer).await;
    assert_eq!(
        item["kind"], "snapshot",
        "a row entering an unheld project has to arrive with the project"
    );
    held.apply(&item);
    assert_eq!(held.threads, ["t-theirs"]);
    assert!(
        held.projects.contains(&"ws-other".to_string()),
        "the client holds the project the row hangs under"
    );

    // And the rebuilt listing left the feed caught up rather than repeating
    // itself on the next poll.
    assert!(
        delivered(&pool, &mut feed, &watcher_viewer)
            .await
            .is_empty()
    );
}

/// A long agent turn is the highest-rate path in the system and the one this
/// design is affordable because of. Nothing it produces is a listing change, so
/// the sidebar does not reorder and the deployment's one counter does not move.
#[sqlx::test]
async fn a_long_agent_turn_does_not_reorder_the_sidebar(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-live", "user-1", Some("ws-live")).await;
    seed_task(&pool, "t-other", "user-1", Some("ws-live")).await;

    let viewer = viewer("user-1");
    let (mut feed, held) = watcher(&pool, &viewer).await;
    let before = reads(&pool).listing_head().await.unwrap().position;

    // Forty messages of agent output and tool results, the shape of one long
    // turn. Deliberately not the turn's start or end — ticket 04 records those,
    // and they are the whole of what a turn contributes.
    for step in 0..20 {
        let uuid = format!("a-{step}");
        seed_agent_message(&pool, "t-live", &uuid, 1, Some("claude-opus-4-5")).await;
        // The call and the result it comes back with, on the turn that made it.
        let work = map_content(
            BlockContext {
                task_id: "t-live",
                message_uuid: &uuid,
                parent_uuid: None,
                message_type: MessageType::Assistant,
                block_index: 1,
            },
            &json!([
                {"type": "tool_use", "id": format!("tu-{step}"), "name": "Bash", "input": {}},
                {"type": "tool_result", "tool_use_id": format!("tu-{step}"), "content": "ok"}
            ]),
        );
        moatless_tasks::message::test_support::seed_activities(&pool, "t-live", &uuid, &work)
            .await
            .unwrap();
    }

    assert_eq!(
        reads(&pool).listing_head().await.unwrap().position,
        before,
        "the agent's own output advanced the deployment's position counter"
    );
    assert!(
        delivered(&pool, &mut feed, &viewer).await.is_empty(),
        "the agent's own output reordered a sidebar"
    );
    assert_eq!(held.threads.len(), 2);
}

// -------------------------------------------------------------------------
// Projects
// -------------------------------------------------------------------------

#[sqlx::test]
async fn renaming_a_project_waits_for_the_next_snapshot(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-live", "user-1", Some("ws-live")).await;

    let viewer = viewer("user-1");
    let (mut feed, held) = watcher(&pool, &viewer).await;
    assert_eq!(held.projects, ["ws-live"]);
    let watched_from = head(&pool).await.position;

    assert!(
        compose::workspaces(pool.clone())
            .update(
                "ws-live",
                &WorkspacePatch {
                    name: Some("Renamed project".into()),
                    ..WorkspacePatch::default()
                },
                &viewer,
            )
            .await
            .unwrap()
    );

    assert_eq!(recorded_after(&pool, watched_from).await, 0);
    assert!(delivered(&pool, &mut feed, &viewer).await.is_empty());

    // Durable, not just delivered: a client opening a fresh subscription reads
    // the same title out of the database.
    let reopened = project(&pool, &viewer, "ws-live").await;
    assert_eq!(reopened["title"], "Renamed project");
}

#[sqlx::test]
async fn creating_and_deleting_a_project_waits_for_the_next_snapshot(pool: PgPool) {
    seed_live_project(&pool).await;

    let viewer = viewer("user-1");
    let (mut feed, held) = watcher(&pool, &viewer).await;
    assert_eq!(held.projects, ["ws-live"]);
    let watched_from = head(&pool).await.position;

    compose::workspaces(pool.clone())
        .insert_workspace(&make_workspace_row("ws-new", "user-1"))
        .await
        .unwrap();

    assert!(
        compose::workspaces(pool.clone())
            .soft_delete("ws-new", &viewer)
            .await
            .unwrap()
    );

    assert_eq!(recorded_after(&pool, watched_from).await, 0);
    assert!(delivered(&pool, &mut feed, &viewer).await.is_empty());
    assert_eq!(Held::opening(&pool, &viewer).await.projects, ["ws-live"]);
}

/// The second way a project leaves the listing, which recorded nothing.
///
/// Overriding a git-declared workspace tombstones it: the row survives so a
/// later sync will not recreate it, `source` flips to `manual`, and `deleted`
/// goes true. `WorkspaceDao::list` still returns it — that is what a
/// tombstone is for — but the shell listing is built from
/// `list_readable_including`, whose readable half excludes it and whose
/// widened half nothing here references. So the group comes off the screen
/// exactly as an ordinary delete takes it off, and until this recorded, it came
/// off with nobody told: the client kept rendering the group until something
/// unrelated moved its listing.
#[sqlx::test]
async fn tombstoning_a_git_declared_project_waits_for_the_next_snapshot(pool: PgPool) {
    let repo = compose::workspaces(pool.clone());
    let mut declared = make_workspace_row("ws-git", "user-1");
    declared.source = moatless_types::workspace::WorkspaceSource::Git;
    declared.source_repository_id = Some("repo-1".to_string());
    declared.source_key = Some("api".to_string());
    repo.insert_workspace(&declared).await.unwrap();

    let viewer = viewer("user-1");
    let (mut feed, held) = watcher(&pool, &viewer).await;
    assert_eq!(held.projects, ["ws-git"]);
    let watched_from = head(&pool).await.position;

    assert!(
        repo.soft_delete_git_origin("ws-git", &viewer)
            .await
            .unwrap()
    );
    assert_eq!(
        recorded_after(&pool, watched_from).await,
        0,
        "a Workspace tombstone advanced the Task listing"
    );
    assert!(delivered(&pool, &mut feed, &viewer).await.is_empty());
    assert!(Held::opening(&pool, &viewer).await.projects.is_empty());
}

/// The same tombstone, with open work still in it.
///
/// A sync tombstones the Workspace a Task is running in — the Task is
/// untouched, and it is what the person is in the middle of. Both listings are
/// built by grouping a conversation under a project the client already holds,
/// so a tombstone that took the group with it took the row out of the sidebar
/// and refused the link to it, with nothing on screen to say why.
#[sqlx::test]
async fn a_tombstoned_project_keeps_the_conversations_still_running_in_it(pool: PgPool) {
    let repo = compose::workspaces(pool.clone());
    let mut declared = make_workspace_row("ws-git", "user-1");
    declared.source = moatless_types::workspace::WorkspaceSource::Git;
    declared.source_repository_id = Some("repo-1".to_string());
    declared.source_key = Some("api".to_string());
    repo.insert_workspace(&declared).await.unwrap();
    seed_task(&pool, "t-running", "user-1", Some("ws-git")).await;

    let viewer = viewer("user-1");
    assert!(
        repo.soft_delete_git_origin("ws-git", &viewer)
            .await
            .unwrap()
    );

    let held = Held::opening(&pool, &viewer).await;
    assert_eq!(held.projects, ["ws-git"]);
    assert_eq!(held.threads, ["t-running"]);
    assert!(
        thread_shell_row(&pool, &viewer, "t-running")
            .await
            .is_some(),
        "the sidebar carries the row, so the link to it must open"
    );
    assert_eq!(
        project(&pool, &viewer, "ws-git").await["scriptsEditable"],
        json!(false),
        "the group is listed for the work still in it, not to be configured"
    );
}

/// Deleting a repository bulk-deletes every workspace its git tree declared,
/// because once the source repo is gone no future sync will prune them. The
/// mutation is Workspace-only, so live clients retain their current groups
/// until a later snapshot replaces the listing.
///
/// Driven through the mounted HTTP route rather than the query underneath it:
/// the statement lives in a handler, and a test that called the handler's
/// helper directly would pass against a route that never reaches it.
#[sqlx::test]
async fn deleting_a_repository_does_not_advance_the_task_listing(pool: PgPool) {
    seed_team(&pool, &format!("personal-{USER_1}")).await;
    compose::seed_repository(&pool, &make_repository_row("repo-gone", USER_1))
        .await
        .unwrap();

    let workspaces = compose::workspaces(pool.clone());
    for key in ["api", "web"] {
        let mut declared = make_workspace_row(&format!("ws-{key}"), USER_1);
        declared.source = moatless_types::workspace::WorkspaceSource::Git;
        declared.source_repository_id = Some("repo-gone".to_string());
        declared.source_key = Some(key.to_string());
        workspaces.insert_workspace(&declared).await.unwrap();
    }
    // A workspace of the viewer's that the repository did not declare, so the
    // assertion below is "the declared ones went" rather than "everything went".
    workspaces
        .insert_workspace(&make_workspace_row("ws-kept", USER_1))
        .await
        .unwrap();

    let viewer = viewer(USER_1);
    let (mut feed, mut held) = watcher(&pool, &viewer).await;
    held.projects.sort();
    assert_eq!(held.projects, ["ws-api", "ws-kept", "ws-web"]);
    let watched_from = head(&pool).await.position;

    let (router, _api) = repository_router(test_app_state(pool.clone()));
    let response = router
        .oneshot(authed_request(
            "DELETE",
            "/api/v1/repositories/repo-gone",
            USER_1,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        recorded_after(&pool, watched_from).await,
        0,
        "repository-driven Workspace deletion advanced the Task listing"
    );

    let items = held.poll(&pool, &mut feed, &viewer).await;
    assert!(
        items.is_empty(),
        "Workspace deletion emitted shell items: {items:#?}"
    );
    assert_eq!(held.projects, ["ws-api", "ws-kept", "ws-web"]);
    assert_eq!(Held::opening(&pool, &viewer).await.projects, ["ws-kept"]);
}

/// A Workspace someone else owns changes without entering the Task feed.
#[sqlx::test]
async fn a_project_change_reaches_only_the_clients_holding_it(pool: PgPool) {
    seed_workspaces(&pool).await;

    let watcher_viewer = viewer("user-1");
    let (mut feed, held) = watcher(&pool, &watcher_viewer).await;
    assert!(
        !held.projects.contains(&"ws-other".to_string()),
        "`ws-other` belongs to `user-2`"
    );
    let watched_from = reads(&pool).listing_head().await.unwrap().position;

    assert!(
        compose::workspaces(pool.clone())
            .update(
                "ws-other",
                &WorkspacePatch {
                    name: Some("Someone else's".into()),
                    ..WorkspacePatch::default()
                },
                &viewer("user-2"),
            )
            .await
            .unwrap()
    );
    assert!(
        compose::workspaces(pool.clone())
            .soft_delete("ws-other", &viewer("user-2"))
            .await
            .unwrap()
    );

    assert_eq!(recorded_after(&pool, watched_from).await, 0);
    assert_eq!(
        delivered(&pool, &mut feed, &watcher_viewer).await,
        Vec::<Value>::new(),
        "a client was told about a project it cannot see"
    );
}

/// A project a viewer cannot read but holds because they follow work inside it
/// is still snapshot-owned. Its mutations emit no live project events.
#[sqlx::test]
async fn a_referenced_project_change_waits_for_the_next_snapshot(pool: PgPool) {
    seed_workspaces(&pool).await;
    let mut theirs = make_task_row("t-theirs", "user-2");
    theirs.workspace_id = Some("ws-other".to_string());
    theirs.visibility = Visibility::Public;
    // `tasks_owner_scope_chk`: a deployment-scoped row has no owner.
    theirs.scope = Scope::Global;
    theirs.owner_user_id = None;
    moatless_tasks::task::test_support::seed_task(&pool, &theirs)
        .await
        .unwrap();
    follow(&pool, "user-1", "t-theirs").await;
    // Something for the narrowing to withhold: a `scripts` assertion against a
    // workspace with no scripts passes whatever the projection does.
    assert!(
        compose::workspaces(pool.clone())
            .update(
                "ws-other",
                &WorkspacePatch {
                    scripts: Some(Some(run_config(json!([{
                        "name": "dev",
                        "command": "bun run secret-thing"
                    }])))),
                    ..WorkspacePatch::default()
                },
                &viewer("user-2"),
            )
            .await
            .unwrap()
    );

    let watcher_viewer = viewer("user-1");
    let (mut feed, mut held) = watcher(&pool, &watcher_viewer).await;
    assert!(
        held.projects.contains(&"ws-other".to_string()),
        "the widening half put an unreadable project in their listing"
    );
    let watched_from = head(&pool).await.position;

    assert!(
        compose::workspaces(pool.clone())
            .update(
                "ws-other",
                &WorkspacePatch {
                    name: Some("Renamed elsewhere".into()),
                    ..WorkspacePatch::default()
                },
                &viewer("user-2"),
            )
            .await
            .unwrap()
    );

    assert!(
        compose::workspaces(pool.clone())
            .soft_delete("ws-other", &viewer("user-2"))
            .await
            .unwrap()
    );
    let items = held.poll(&pool, &mut feed, &watcher_viewer).await;
    assert!(
        items.is_empty(),
        "Workspace changes emitted shell items: {items:#?}"
    );
    assert_eq!(recorded_after(&pool, watched_from).await, 0);
    assert!(held.projects.contains(&"ws-other".to_string()));
    assert!(
        !Held::opening(&pool, &watcher_viewer)
            .await
            .projects
            .contains(&"ws-other".to_string())
    );
}
