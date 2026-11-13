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
use crate::common::make_repository_row;
use crate::common::make_task_row;
use crate::common::make_workspace_row;
use crate::common::run_config;
use crate::common::seed_team;
use crate::common::test_app_state;
use axum::http::StatusCode;
use futures::future::BoxFuture;
use moatless_tasks::activities::mapping::BlockContext;
use moatless_tasks::activities::mapping::map_content;
use moatless_tasks::message::models::MessageType;
use moatless_tasks::tag::test_support as tag_fixtures;
use moatless_tasks::task::models::TaskPatch;
use moatless_tasks::task::models::TaskStatus;
use moatless_tasks::task::models::Visibility;
use moatless_types::access_control::Scope;
use moatless_types::access_control::ScopeContext;
use moatless_workspaces::compose;
use moatless_workspaces::repository::routes::repository_router;
use moatless_workspaces::workspace::UpdateOutcome;
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

/// A removal must name only a conversation this client was actually sent.
///
/// It must never leak the id of another viewer's unfollowed, private, or
/// deleted conversation; the watcher here holds one of its own, so an
/// implementation that only emits removals when idle would still fail.
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

/// Closing must tell a watching client via the updated row, not a removal.
///
/// The conversation hasn't left the viewer's world — it's still followed and
/// readable — so a removal would overstate it and wouldn't survive a resumed
/// feed the way the row does.
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

/// Losing read access is the fourth way a row leaves, and the one no mutation
/// of the conversation's own row expresses: the follow survives, the
/// conversation survives, and it still has to go.
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
                    // Sent, not seeded: `seed_message` writes a frozen row, and
                    // only the live projection records a listing change.
                    moatless_tasks::message::test_support::queue_named(
                        &pool,
                        &id,
                        &format!("{id}-said"),
                        "hello",
                    )
                    .await;
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
                    // Projected, not seeded, for the reason the case above is.
                    moatless_tasks::message::test_support::seed_agent_blocks(
                        &pool,
                        &id,
                        &format!("{id}-session"),
                        Some(1),
                        json!([
                            {"type": "text", "text": "Which one?"},
                            {"type": "tool_use", "id": "tu-1", "name": "AskUserQuestion", "input": {}}
                        ]),
                    )
                    .await;
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

/// Message-send follows `message::handlers`' order: follow first, write second.
///
/// The two changes land between the same poll pair and coalesce into one
/// re-read, so the client gets the row and the message together, never the
/// message against a row it doesn't hold.
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

/// A conversation entering a project the client has never held can't be a
/// delta.
///
/// Two events at one position collapse into one, so the feed sends the listing
/// whole instead of a bare `thread-upserted` a real client would silently drop.
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
                source: &uuid,
                source_block: 0,
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

/// A V1 client has no project item, so a rename reaches it as the listing.
#[sqlx::test]
async fn renaming_a_project_sends_a_v1_client_the_listing(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-live", "user-1", Some("ws-live")).await;

    let viewer = viewer("user-1");
    let (mut feed, held) = watcher(&pool, &viewer).await;
    assert_eq!(held.projects, ["ws-live"]);
    let watched_from = head(&pool).await.position;

    assert_eq!(
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
            .unwrap(),
        UpdateOutcome::Applied
    );

    assert_eq!(recorded_after(&pool, watched_from).await, 1);
    let items = delivered(&pool, &mut feed, &viewer).await;
    assert_eq!(items.len(), 1, "expected one rebuild, got {items:#?}");
    assert_eq!(items[0]["kind"], "snapshot");
    assert_eq!(
        items[0]["snapshot"]["projects"][0]["title"],
        "Renamed project"
    );

    // Durable, not just delivered: a client opening a fresh subscription reads
    // the same title out of the database.
    let reopened = project(&pool, &viewer, "ws-live").await;
    assert_eq!(reopened["title"], "Renamed project");
}

/// Created and deleted between two polls: the client never held it, so the
/// deletion it last saw leaves nothing to say.
#[sqlx::test]
async fn a_project_created_and_deleted_between_polls_sends_nothing(pool: PgPool) {
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

    assert_eq!(recorded_after(&pool, watched_from).await, 2);
    assert!(delivered(&pool, &mut feed, &viewer).await.is_empty());
    assert_eq!(Held::opening(&pool, &viewer).await.projects, ["ws-live"]);
}

/// Tombstoning a git-declared workspace keeps the row but leaves it out of
/// `list_readable_including`, so a client holding it is sent a listing
/// without it.
#[sqlx::test]
async fn tombstoning_a_git_declared_project_takes_it_off_the_listing(pool: PgPool) {
    let repo = compose::workspaces(pool.clone());
    let mut declared = make_workspace_row("ws-git", "user-1");
    declared.source = moatless_workspace_types::workspace::WorkspaceSource::Git;
    declared.source_repository_id = Some("repo-1".to_string());
    declared.source_key = Some("api".to_string());
    repo.insert_workspace(&declared).await.unwrap();

    let viewer = viewer("user-1");
    let (mut feed, mut held) = watcher(&pool, &viewer).await;
    assert_eq!(held.projects, ["ws-git"]);
    let watched_from = head(&pool).await.position;

    assert!(repo.soft_delete("ws-git", &viewer).await.unwrap());
    assert_eq!(recorded_after(&pool, watched_from).await, 1);
    let items = held.poll(&pool, &mut feed, &viewer).await;
    assert_eq!(items.len(), 1, "expected one rebuild, got {items:#?}");
    assert!(held.projects.is_empty());
}

/// Tombstoning a Workspace leaves any Task running in it untouched.
///
/// Both listings group a conversation under a project the client already holds,
/// so if the tombstone took the group with it, the row would vanish from the
/// sidebar and refuse its link with no explanation.
#[sqlx::test]
async fn a_tombstoned_project_keeps_the_conversations_still_running_in_it(pool: PgPool) {
    let repo = compose::workspaces(pool.clone());
    let mut declared = make_workspace_row("ws-git", "user-1");
    declared.source = moatless_workspace_types::workspace::WorkspaceSource::Git;
    declared.source_repository_id = Some("repo-1".to_string());
    declared.source_key = Some("api".to_string());
    repo.insert_workspace(&declared).await.unwrap();
    seed_task(&pool, "t-running", "user-1", Some("ws-git")).await;

    let viewer = viewer("user-1");
    assert!(repo.soft_delete("ws-git", &viewer).await.unwrap());

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
/// one recorded change each, and a client holding them is sent a listing
/// without them.
///
/// Driven through the mounted HTTP route, not the query underneath, since the
/// delete statement lives in the handler and a direct call would miss it.
#[sqlx::test]
async fn deleting_a_repository_takes_its_workspaces_off_the_listing(pool: PgPool) {
    seed_team(&pool, &format!("personal-{USER_1}")).await;
    compose::seed_repository(&pool, &make_repository_row("repo-gone", USER_1))
        .await
        .unwrap();

    let workspaces = compose::workspaces(pool.clone());
    for key in ["api", "web"] {
        let mut declared = make_workspace_row(&format!("ws-{key}"), USER_1);
        declared.source = moatless_workspace_types::workspace::WorkspaceSource::Git;
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
    assert_eq!(recorded_after(&pool, watched_from).await, 2);

    let items = held.poll(&pool, &mut feed, &viewer).await;
    assert_eq!(items.len(), 1, "expected one rebuild, got {items:#?}");
    assert_eq!(items[0]["kind"], "snapshot");
    assert_eq!(held.projects, ["ws-kept"]);
}

/// A Workspace someone else owns changes, recorded, and a client that can
/// neither read nor hold it is told nothing.
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

    assert_eq!(
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
            .unwrap(),
        UpdateOutcome::Applied
    );
    assert!(
        compose::workspaces(pool.clone())
            .soft_delete("ws-other", &viewer("user-2"))
            .await
            .unwrap()
    );

    assert_eq!(recorded_after(&pool, watched_from).await, 2);
    assert_eq!(
        delivered(&pool, &mut feed, &watcher_viewer).await,
        Vec::<Value>::new(),
        "a client was told about a project it cannot see"
    );
}

/// A project a viewer cannot read but holds because they follow work inside it
/// changes by snapshot, never by a project item: the snapshot keeps it listed,
/// narrowed, for as long as the followed conversation names it.
#[sqlx::test]
async fn a_referenced_project_change_is_sent_as_a_snapshot(pool: PgPool) {
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
    assert_eq!(
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
            .unwrap(),
        UpdateOutcome::Applied
    );

    let watcher_viewer = viewer("user-1");
    let (mut feed, mut held) = watcher(&pool, &watcher_viewer).await;
    assert!(
        held.projects.contains(&"ws-other".to_string()),
        "the widening half put an unreadable project in their listing"
    );
    let watched_from = head(&pool).await.position;

    assert_eq!(
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
            .unwrap(),
        UpdateOutcome::Applied
    );

    assert!(
        compose::workspaces(pool.clone())
            .soft_delete("ws-other", &viewer("user-2"))
            .await
            .unwrap()
    );
    assert_eq!(recorded_after(&pool, watched_from).await, 2);
    let items = held.poll(&pool, &mut feed, &watcher_viewer).await;
    assert_eq!(items.len(), 1, "expected one rebuild, got {items:#?}");
    assert_eq!(items[0]["kind"], "snapshot");
    // Still listed after the delete: the conversation user-1 follows names it.
    assert!(held.projects.contains(&"ws-other".to_string()));
    let sent = items[0]["snapshot"]["projects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|project| project["id"] == "ws-other")
        .unwrap();
    assert_eq!(sent["title"], "Renamed elsewhere");
    assert_eq!(
        sent["scripts"],
        json!([]),
        "the narrowing withheld the scripts"
    );
}
