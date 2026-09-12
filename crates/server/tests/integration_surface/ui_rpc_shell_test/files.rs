//! Addressing a conversation's files.
//!
//! The client asks for files by the `cwd` it holds, and the only `cwd` it has
//! for a conversation is the `worktreePath` this module put in the listing. So
//! the two have to be each other's inverse, and the round trip is only real
//! against a database: the reverse direction re-reads the conversation through
//! the same follow and access filters the listing applies, which are SQL.
//!
//! What a Sandbox answers is a gRPC round trip against a pod, which no test here
//! has. These stop where that begins — at the point the request has been
//! addressed and authorized and is about to leave for the Sandbox — because
//! everything before it is this module's and everything after it is not.

use super::harness::{NoSandboxes, reads, snapshot, thread, viewer};
use super::seeding::{seed_sandbox, seed_task};
use crate::common::make_workspace_row;
use moatless_t3code::Sources;
use moatless_t3code::files;
use moatless_types::access_control::ScopeContext;
use moatless_workspaces::compose;
use sqlx::PgPool;

// =========================================================================
// Addressing a conversation's files
// =========================================================================
//
// The client asks for files by the `cwd` it holds, and the only `cwd` it has
// for a conversation is the `worktreePath` this module put in the listing. So
// the two have to be each other's inverse, and the round trip is only real
// against a database: the reverse direction re-reads the conversation through
// the same follow and access filters the listing applies, which are SQL.
//
// What a Sandbox answers is a gRPC round trip against a pod, which no test here
// has. These stop where that begins — at the point the request has been
// addressed and authorized and is about to leave for the Sandbox — because
// everything before it is this module's and everything after it is not.

/// The `worktreePath` in a viewer's listing is the `cwd` that reaches that same
/// conversation's Sandbox, and only for a viewer whose listing contains it.
#[sqlx::test]
async fn the_listings_worktree_path_addresses_that_conversations_files(pool: PgPool) {
    compose::workspaces(pool.clone())
        .insert_workspace(&make_workspace_row("ws-files", "user-1"))
        .await
        .unwrap();
    seed_task(&pool, "t-files", "user-1", Some("ws-files")).await;
    seed_sandbox(&pool, "t-files", "running", true).await;

    let owner = viewer("user-1");
    let cwd = thread(&pool, &owner, "t-files").await["worktreePath"]
        .as_str()
        .expect("a running sandbox reports a worktree")
        .to_string();

    // Reached the Sandbox: the stub is the only thing that refused it.
    let error = files::list_entries(
        &reads(&pool),
        Sources {
            sandboxes: &NoSandboxes,
            snapshots: None,
        },
        &owner,
        &cwd,
    )
    .await
    .expect_err("the stub connector has no sandbox to answer with");
    assert_eq!(
        error.message(),
        "The sandbox for this conversation is not running.",
        "the cwd resolved to the conversation and the viewer was allowed it"
    );

    // Same path, a viewer whose listing does not contain the conversation. A
    // `cwd` is a string the client sends, so this is the only thing standing
    // between a guessed Task id and someone else's repository.
    let outsider = ScopeContext {
        user_id: "user-2".into(),
        is_admin: false,
    };
    let error = files::read_file(
        &reads(&pool),
        Sources {
            sandboxes: &NoSandboxes,
            snapshots: None,
        },
        &outsider,
        &cwd,
        "README.md",
    )
    .await
    .expect_err("a conversation they do not follow is not theirs to read");
    assert_eq!(
        error.message(),
        "This conversation's workspace is not available.",
        "and it must not say whether the conversation exists"
    );
}

/// A conversation with no Sandbox reports no worktree, so the client falls back
/// to the project's root — and that has to come back as the state it is rather
/// than as a path nothing recognizes.
#[sqlx::test]
async fn a_project_root_is_answered_as_a_conversation_with_no_sandbox(pool: PgPool) {
    compose::workspaces(pool.clone())
        .insert_workspace(&make_workspace_row("ws-bare", "user-1"))
        .await
        .unwrap();
    seed_task(&pool, "t-bare", "user-1", Some("ws-bare")).await;
    sqlx::query("UPDATE tasks SET sandbox_id = NULL WHERE task_id = $1")
        .bind("t-bare")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM sandboxes WHERE sandbox_id = $1")
        .bind("t-bare")
        .execute(&pool)
        .await
        .unwrap();

    let owner = viewer("user-1");
    let listing = snapshot(&pool, &owner).await;
    assert!(
        listing["threads"][0]["worktreePath"].is_null(),
        "no sandbox was ever asked for"
    );
    let cwd = listing["projects"][0]["workspaceRoot"]
        .as_str()
        .unwrap()
        .to_string();

    let error = files::list_entries(
        &reads(&pool),
        Sources {
            sandboxes: &NoSandboxes,
            snapshots: None,
        },
        &owner,
        &cwd,
    )
    .await
    .expect_err("a project has no files of its own");
    assert_eq!(
        error.message(),
        "This conversation has no sandbox, so it has no workspace yet."
    );
}
