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
use moatless_vibe_backend::loops::sync::parse_loop;
use moatless_vibe_backend::loops::sync::sync_loops;
use moatless_vibe_backend::repository::dao as repository_dao;

use common::{make_repository_row, test_app_state};

fn auth(user_id: &str) -> AuthUser {
    AuthUser {
        // `AuthUser::user_id` is a `Uuid`; derive a stable one from the symbolic
        // name. Its value is immaterial here — connections resolve by GitHub org
        // and run-as falls back to the repo owner, neither keyed on this id.
        user_id: uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, user_id.as_bytes()),
        login: user_id.to_string(),
        role: None,
        provider: Some("github".to_string()),
        group_id: None,
        team_ids: vec![],
        task_id: None,
        is_agent_context: false,
    }
}

/// Insert the personal team a repository references, satisfying the
/// `repositories_team_id_fkey` FK. Idempotent. `make_repository_row` sets
/// `team_id = personal-{user_id}`.
async fn seed_team(pool: &PgPool, user_id: &str) {
    sqlx::query(
        "INSERT INTO teams (id, name, slug) VALUES ($1, $2, $1) \
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(format!("personal-{user_id}"))
    .bind(format!("{user_id} Personal"))
    .execute(pool)
    .await
    .unwrap();
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

async fn insert_slack_connection(pool: &PgPool, team_id: &str) {
    sqlx::query(
        "INSERT INTO integration_connections \
         (id, adapter_kind, connection_kind, external_account_id, created_by, scope) \
         VALUES ($1, 'slack', 'slack_bot', $2, 'user-1', 'user')",
    )
    .bind(format!("conn_{team_id}"))
    .bind(team_id)
    .execute(pool)
    .await
    .unwrap();
}

async fn insert_linear_connection(pool: &PgPool, org_id: &str) {
    sqlx::query(
        "INSERT INTO integration_connections \
         (id, adapter_kind, connection_kind, external_account_id, created_by, scope) \
         VALUES ($1, 'linear', 'linear_oauth', $2, 'user-1', 'user')",
    )
    .bind(format!("conn_{org_id}"))
    .bind(org_id)
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

const GITHUB_LOOP: &str =
    "---\non: github_pr\nagentType: claude-code\ntags: [review]\n---\n\nReview this PR.\n";
const SCHEDULE_LOOP: &str = "---\non: schedule\nschedule:\n  cron: \"0 0 6 * * MON-FRI *\"\n  timezone: Europe/Stockholm\n  message: \"Run the daily check.\"\n---\n\nDaily health check.\n";
const SLACK_LOOP: &str = "---\non: slack\nsourceMatcher: C0000000001\nteamId: T0000000001\nroutingMode: ongoing\nadapterConfig:\n  allow_bot_messages: true\n  trigger: all_messages\n---\n\nTriage CI failures posted to #notifications-cicd.\n";
const LINEAR_LOOP: &str = "---\non: linear\nsourceMatcher: team-uuid-eng\ntriggerStates: [Ready]\nprojectIds: [proj-launch]\nstartState: In Progress\n---\n\nPick up the Linear ticket and implement it.\n";

async fn count_manual_triggers(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM triggers WHERE managed_by IS NULL")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test]
async fn sync_creates_then_reconciles_loops(pool: PgPool) {
    let repo = make_repository_row("repo-loops", "user-1");
    seed_team(&pool, "user-1").await;
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
    assert!(
        summary.skipped.is_empty(),
        "nothing skipped: {:?}",
        summary.skipped
    );

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
    assert_eq!(
        gh.get::<Option<String>, _>("name").as_deref(),
        Some("java-review")
    );
    assert_eq!(gh.get::<String, _>("agent_type"), "claude-code");
    assert_eq!(
        gh.get::<Option<String>, _>("prompt").as_deref(),
        Some("Review this PR.")
    );
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
    assert_eq!(
        sched.get::<String, _>("cron_expression"),
        "0 0 6 * * MON-FRI *"
    );
    assert_eq!(sched.get::<String, _>("timezone"), "Europe/Stockholm");
    assert_eq!(
        sched.get::<String, _>("message_template"),
        "Run the daily check."
    );

    assert_eq!(
        count_manual_triggers(&pool).await,
        1,
        "manual trigger untouched"
    );

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
    assert_eq!(
        count_manual_triggers(&pool).await,
        1,
        "manual trigger still untouched"
    );
}

#[sqlx::test]
async fn github_pr_loop_without_connection_is_skipped(pool: PgPool) {
    let repo = make_repository_row("repo-noconn", "user-1");
    seed_team(&pool, "user-1").await;
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

#[sqlx::test]
async fn sync_creates_slack_loop(pool: PgPool) {
    let repo = make_repository_row("repo-slack", "user-1");
    seed_team(&pool, "user-1").await;
    repository_dao::insert(&pool, &repo).await.unwrap();
    insert_slack_connection(&pool, "T0000000001").await;

    let state = test_app_state(pool.clone());
    let auth = auth("user-1");

    let loops = vec![parse_loop(SLACK_LOOP, "e2e-triage").unwrap()];
    let summary = sync_loops(&state, &repo, &auth, loops).await.unwrap();
    assert_eq!(summary.created, 1, "slack loop created");
    assert_eq!(summary.updated, 0);
    assert!(
        summary.skipped.is_empty(),
        "nothing skipped: {:?}",
        summary.skipped
    );

    // slack loop -> adapter_event trigger pointed at the channel via the
    // team's connection, with the ongoing routing mode flowing through.
    let row = sqlx::query(
        "SELECT t.kind, r.routing_mode, aes.adapter_kind, aes.source_matcher, \
                aes.connection_id, aes.adapter_config \
         FROM triggers t \
         JOIN trigger_rules r ON r.trigger_id = t.id \
         JOIN adapter_event_trigger_sources aes ON aes.trigger_id = t.id \
         WHERE t.source_repository_id = $1 AND t.loop_name = 'e2e-triage' AND t.managed_by = 'repo_loop'",
    )
    .bind(&repo.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row.get::<String, _>("kind"), "adapter_event");
    assert_eq!(row.get::<String, _>("routing_mode"), "ongoing");
    assert_eq!(row.get::<String, _>("adapter_kind"), "slack");
    assert_eq!(row.get::<String, _>("source_matcher"), "C0000000001");
    assert_eq!(row.get::<String, _>("connection_id"), "conn_T0000000001");
    let cfg: serde_json::Value = row.get("adapter_config");
    assert_eq!(
        cfg.get("allow_bot_messages")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        cfg.get("trigger").and_then(serde_json::Value::as_str),
        Some("all_messages")
    );
}

#[sqlx::test]
async fn sync_creates_linear_loop(pool: PgPool) {
    let repo = make_repository_row("repo-linear", "user-1");
    seed_team(&pool, "user-1").await;
    repository_dao::insert(&pool, &repo).await.unwrap();
    // Single linear connection — resolved without an explicit teamId.
    insert_linear_connection(&pool, "org-acme").await;

    let state = test_app_state(pool.clone());
    let auth = auth("user-1");

    let loops = vec![parse_loop(LINEAR_LOOP, "ready-pool").unwrap()];
    let summary = sync_loops(&state, &repo, &auth, loops).await.unwrap();
    assert_eq!(summary.created, 1, "linear loop created");
    assert!(
        summary.skipped.is_empty(),
        "nothing skipped: {:?}",
        summary.skipped
    );

    // linear loop -> adapter_event trigger pointed at the team uuid via the
    // single linear connection, with status/project filters in adapter_config
    // and create_on_status_entry forced on.
    let row = sqlx::query(
        "SELECT t.kind, aes.adapter_kind, aes.source_matcher, \
                aes.connection_id, aes.adapter_config \
         FROM triggers t \
         JOIN adapter_event_trigger_sources aes ON aes.trigger_id = t.id \
         WHERE t.source_repository_id = $1 AND t.loop_name = 'ready-pool' AND t.managed_by = 'repo_loop'",
    )
    .bind(&repo.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row.get::<String, _>("kind"), "adapter_event");
    assert_eq!(row.get::<String, _>("adapter_kind"), "linear");
    assert_eq!(row.get::<String, _>("source_matcher"), "team-uuid-eng");
    assert_eq!(row.get::<String, _>("connection_id"), "conn_org-acme");
    let cfg: serde_json::Value = row.get("adapter_config");
    assert_eq!(
        cfg.get("create_on_status_entry")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        cfg.get("trigger_states"),
        Some(&serde_json::json!(["Ready"]))
    );
    assert_eq!(
        cfg.get("project_ids"),
        Some(&serde_json::json!(["proj-launch"]))
    );
    assert_eq!(
        cfg.get("start_state"),
        Some(&serde_json::json!("In Progress"))
    );
}

#[sqlx::test]
async fn linear_loop_without_connection_is_skipped(pool: PgPool) {
    let repo = make_repository_row("repo-linear-noconn", "user-1");
    seed_team(&pool, "user-1").await;
    repository_dao::insert(&pool, &repo).await.unwrap();
    // No linear connection inserted.

    let state = test_app_state(pool.clone());
    let auth = auth("user-1");

    let loops = vec![parse_loop(LINEAR_LOOP, "ready-pool").unwrap()];
    let summary = sync_loops(&state, &repo, &auth, loops).await.unwrap();

    assert_eq!(summary.created, 0);
    assert_eq!(summary.skipped.len(), 1, "linear loop skipped");
    assert!(
        summary.skipped[0].reason.contains("linear connection"),
        "reason: {}",
        summary.skipped[0].reason
    );
}

#[sqlx::test]
async fn slack_loop_without_connection_is_skipped(pool: PgPool) {
    let repo = make_repository_row("repo-slack-noconn", "user-1");
    seed_team(&pool, "user-1").await;
    repository_dao::insert(&pool, &repo).await.unwrap();
    // No slack connection inserted for team T0000000001.

    let state = test_app_state(pool.clone());
    let auth = auth("user-1");

    let loops = vec![parse_loop(SLACK_LOOP, "e2e-triage").unwrap()];
    let summary = sync_loops(&state, &repo, &auth, loops).await.unwrap();

    assert_eq!(summary.created, 0);
    assert_eq!(summary.updated, 0);
    assert_eq!(summary.skipped.len(), 1, "slack loop skipped");
    assert_eq!(summary.skipped[0].loop_name, "e2e-triage");
    assert!(
        summary.skipped[0].reason.contains("slack connection"),
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

/// Re-syncing a managed loop must not wipe `ongoing_task_id` — that column is
/// mutable runtime state set by firing, not by the repo declaration. Regression
/// guard for the delete-then-insert reconcile path in `upsert_managed_loop`.
#[sqlx::test]
async fn resync_preserves_ongoing_task_id(pool: PgPool) {
    let repo = make_repository_row("repo-ongoing", "user-1");
    seed_team(&pool, "user-1").await;
    repository_dao::insert(&pool, &repo).await.unwrap();
    insert_github_connection(&pool, "test").await;

    let state = test_app_state(pool.clone());
    let auth = auth("user-1");

    // First sync creates the managed loop.
    let loops = vec![parse_loop(GITHUB_LOOP, "java-review").unwrap()];
    let summary = sync_loops(&state, &repo, &auth, loops).await.unwrap();
    assert_eq!(summary.created, 1);

    // Simulate a firing linking an ongoing task to the loop's rule row.
    let trigger_id: String = sqlx::query_scalar(
        "SELECT id FROM triggers \
         WHERE source_repository_id = $1 AND loop_name = 'java-review' AND managed_by = 'repo_loop'",
    )
    .bind(&repo.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE trigger_rules SET ongoing_task_id = 'task-ongoing-1' WHERE trigger_id = $1",
    )
    .bind(&trigger_id)
    .execute(&pool)
    .await
    .unwrap();

    // Re-sync the same loop; the reconciler must preserve the runtime link.
    let loops = vec![parse_loop(GITHUB_LOOP, "java-review").unwrap()];
    let summary = sync_loops(&state, &repo, &auth, loops).await.unwrap();
    assert_eq!(summary.updated, 1, "loop updated in place");

    let ongoing: Option<String> =
        sqlx::query_scalar("SELECT ongoing_task_id FROM trigger_rules WHERE trigger_id = $1")
            .bind(&trigger_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        ongoing.as_deref(),
        Some("task-ongoing-1"),
        "re-sync must preserve the ongoing task link"
    );
}
