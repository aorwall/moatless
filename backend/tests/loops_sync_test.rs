#![cfg(feature = "db-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr
)]
//! DB-backed integration tests for repo "loop" sync (`sync_loops`).
//!
//! These bypass the GitHub fetcher and call `sync_loops` with already-parsed
//! loops, so they exercise resolution + upsert + reconcile-delete against a
//! real Postgres without needing a live repo fixture.

mod common;

use sqlx::{PgPool, Row};

use moatless_vibe_backend::extractors::AuthUser;
use moatless_vibe_backend::loops::parser::parse_loop;
use moatless_vibe_backend::loops::sync::sync_loops;
use moatless_vibe_backend::repository::dao as repository_dao;

use common::{make_repository_row, test_app_state};

fn auth(user_id: &str) -> AuthUser {
    AuthUser {
        user_id: user_id.to_string(),
        login: user_id.to_string(),
        role: None,
        provider: Some("github".to_string()),
        group_id: None,
        team_ids: vec![],
        task_id: None,
        is_agent_context: false,
    }
}

async fn insert_github_connection(pool: &PgPool, owner: &str) {
    sqlx::query(
        "INSERT INTO integration_connections \
         (id, adapter_kind, connection_kind, external_account_id, created_by, scope) \
         VALUES ($1, 'github', 'github_app', $2, 'user-1', 'user')",
    )
    .bind(format!("conn_{owner}"))
    .bind(owner)
    .execute(pool)
    .await
    .unwrap();
}

/// Insert a manual (non-loop) trigger on a repo so we can assert reconcile
/// never touches it.
async fn insert_manual_trigger(pool: &PgPool, repo_id: &str) {
    sqlx::query(
        "INSERT INTO triggers (id, name, kind, active, scope, created_by) \
         VALUES ('tr_manual_1', 'manual one', 'adapter_event', true, 'user', 'user-1')",
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO trigger_rules (trigger_id, repository_id, routing_mode, agent_type) \
         VALUES ('tr_manual_1', $1, 'by_subject', 'claude-code')",
    )
    .bind(repo_id)
    .execute(pool)
    .await
    .unwrap();
}

const GITHUB_LOOP: &str = "---\non: github_pr\nagentType: claude-code\ntags: [review]\n---\n\nReview this PR.\n";
const SCHEDULE_LOOP: &str = "---\non: schedule\nschedule:\n  cron: \"0 0 6 * * MON-FRI *\"\n  timezone: Europe/Stockholm\n  message: \"Run the daily check.\"\n---\n\nDaily health check.\n";

async fn count_manual_triggers(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM triggers WHERE managed_by IS NULL")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test]
async fn sync_creates_then_reconciles_loops(pool: PgPool) {
    let repo = make_repository_row("repo-loops", "user-1");
    repository_dao::insert(&pool, &repo).await.unwrap();
    // make_repository_row uses https://github.com/test/repo.git -> org "test".
    insert_github_connection(&pool, "test").await;
    insert_manual_trigger(&pool, &repo.id).await;

    let state = test_app_state(pool.clone());
    let auth = auth("user-1");

    let loops = vec![
        parse_loop(GITHUB_LOOP, "java-review").unwrap(),
        parse_loop(SCHEDULE_LOOP, "daily-health-check").unwrap(),
    ];

    let summary = sync_loops(&state, &repo, &auth, loops).await.unwrap();
    assert_eq!(summary.created, 2, "both loops created");
    assert_eq!(summary.updated, 0);
    assert_eq!(summary.removed, 0);
    assert!(summary.skipped.is_empty(), "nothing skipped: {:?}", summary.skipped);

    // github_pr loop -> adapter_event trigger pointed at owner/repo.
    let gh = sqlx::query(
        "SELECT t.kind, t.name, r.agent_type, r.prompt, aes.adapter_kind, aes.source_matcher \
         FROM triggers t \
         JOIN trigger_rules r ON r.trigger_id = t.id \
         JOIN adapter_event_trigger_sources aes ON aes.trigger_id = t.id \
         WHERE t.source_repository_id = $1 AND t.loop_name = 'java-review' AND t.managed_by = 'repo_loop'",
    )
    .bind(&repo.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(gh.get::<String, _>("kind"), "adapter_event");
    assert_eq!(gh.get::<Option<String>, _>("name").as_deref(), Some("java-review"));
    assert_eq!(gh.get::<String, _>("agent_type"), "claude-code");
    assert_eq!(gh.get::<Option<String>, _>("prompt").as_deref(), Some("Review this PR."));
    assert_eq!(gh.get::<String, _>("adapter_kind"), "github");
    assert_eq!(gh.get::<String, _>("source_matcher"), "test/repo");

    // schedule loop -> schedule trigger with cron + message.
    let sched = sqlx::query(
        "SELECT t.kind, sts.cron_expression, sts.timezone, sts.message_template \
         FROM triggers t \
         JOIN schedule_trigger_sources sts ON sts.trigger_id = t.id \
         WHERE t.source_repository_id = $1 AND t.loop_name = 'daily-health-check' AND t.managed_by = 'repo_loop'",
    )
    .bind(&repo.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(sched.get::<String, _>("kind"), "schedule");
    assert_eq!(sched.get::<String, _>("cron_expression"), "0 0 6 * * MON-FRI *");
    assert_eq!(sched.get::<String, _>("timezone"), "Europe/Stockholm");
    assert_eq!(sched.get::<String, _>("message_template"), "Run the daily check.");

    assert_eq!(count_manual_triggers(&pool).await, 1, "manual trigger untouched");

    // Re-sync dropping the github_pr loop: it should be removed, schedule kept,
    // manual trigger still untouched.
    let loops = vec![parse_loop(SCHEDULE_LOOP, "daily-health-check").unwrap()];
    let summary = sync_loops(&state, &repo, &auth, loops).await.unwrap();
    assert_eq!(summary.created, 0);
    assert_eq!(summary.updated, 1, "schedule loop updated in place");
    assert_eq!(summary.removed, 1, "java-review loop removed");

    let remaining: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM triggers WHERE source_repository_id = $1 AND managed_by = 'repo_loop'",
    )
    .bind(&repo.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(remaining, 1, "only the schedule loop remains");
    assert_eq!(count_manual_triggers(&pool).await, 1, "manual trigger still untouched");
}

#[sqlx::test]
async fn github_pr_loop_without_connection_is_skipped(pool: PgPool) {
    let repo = make_repository_row("repo-noconn", "user-1");
    repository_dao::insert(&pool, &repo).await.unwrap();
    // No github connection inserted for org "test".

    let state = test_app_state(pool.clone());
    let auth = auth("user-1");

    let loops = vec![parse_loop(GITHUB_LOOP, "java-review").unwrap()];
    let summary = sync_loops(&state, &repo, &auth, loops).await.unwrap();

    assert_eq!(summary.created, 0);
    assert_eq!(summary.updated, 0);
    assert_eq!(summary.skipped.len(), 1, "github_pr loop skipped");
    assert_eq!(summary.skipped[0].loop_name, "java-review");
    assert!(
        summary.skipped[0].reason.contains("no github connection"),
        "reason: {}",
        summary.skipped[0].reason
    );

    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM triggers WHERE source_repository_id = $1 AND managed_by = 'repo_loop'",
    )
    .bind(&repo.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 0, "no trigger created for skipped loop");
}
