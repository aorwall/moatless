#![cfg(feature = "db-tests")]
#![expect(clippy::unwrap_used, reason = "tests fail by panicking")]
//! DB-backed integration tests for repo "loop" sync (`sync_loops`).
//!
//! These bypass the GitHub fetcher and call `sync_loops` with already-parsed
//! loops, so they exercise resolution + upsert + reconcile-delete against a
//! real Postgres without needing a live repo fixture.

use crate::common;

use chrono::Utc;
use sqlx::{PgPool, Row};

use moatless_core::http_auth::AuthUser;
use moatless_loops::loops::models::ManagedLoopUpsert;
use moatless_loops::loops::models::NewScheduleSource;
use moatless_loops::loops::sync::parse_loop;
use moatless_loops::loops::test_support::{activate_loop, insert_manual_loop};
use moatless_types::access_control::ScopeContext;
use moatless_types::loops::RoutingMode;
use moatless_workspaces::compose;
use moatless_workspaces::repository::models::Repository;

use common::{make_repository_row, test_app_state};

fn auth(user_id: &str) -> AuthUser {
    AuthUser {
        // `AuthUser::user_id` is a `Uuid`; derive a stable one from the symbolic
        // name. Its value is immaterial here — connections resolve by GitHub
        // organization and Git sync never assigns a run-as identity.
        user_id: uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, user_id.as_bytes()),
        login: user_id.to_string(),
        role: None,
        provider: Some("github".to_string()),
        task_id: None,
        is_agent_context: false,
    }
}

async fn insert_github_connection(pool: &PgPool, owner: &str) {
    sqlx::query(
        "INSERT INTO adapter_connections \
         (id, adapter_kind, connection_kind, external_account_id, created_by, scope, owner_user_id) \
         VALUES ($1, 'github', 'github_app', $2, 'user-1', 'user', 'user-1')",
    )
    .bind(format!("conn_{owner}"))
    .bind(owner)
    .execute(pool)
    .await
    .unwrap();
}

async fn insert_slack_connection(pool: &PgPool, team_id: &str) {
    sqlx::query(
        "INSERT INTO adapter_connections \
         (id, adapter_kind, connection_kind, external_account_id, created_by, scope, owner_user_id) \
         VALUES ($1, 'slack', 'slack_bot', $2, 'user-1', 'user', 'user-1')",
    )
    .bind(format!("conn_{team_id}"))
    .bind(team_id)
    .execute(pool)
    .await
    .unwrap();
}

async fn insert_linear_connection(pool: &PgPool, org_id: &str) {
    sqlx::query(
        "INSERT INTO adapter_connections \
         (id, adapter_kind, connection_kind, external_account_id, created_by, scope, owner_user_id) \
         VALUES ($1, 'linear', 'linear_oauth', $2, 'user-1', 'user', 'user-1')",
    )
    .bind(format!("conn_{org_id}"))
    .bind(org_id)
    .execute(pool)
    .await
    .unwrap();
}

const GITHUB_LOOP: &str = "---\non: github_pr\nname: Java Review\nagentProfile: claude-code\ntags: [review]\nskills: [java-review, e2e-testing]\n---\n\nReview this PR.\n";
const SCHEDULE_LOOP: &str = "---\non: schedule\nname: Daily Health Check\nschedule:\n  cron: \"0 0 6 * * MON-FRI *\"\n  timezone: Europe/Stockholm\n  message: \"Run the daily check.\"\n---\n\nDaily health check.\n";
const SLACK_LOOP: &str = "---\non: slack\nname: E2E Triage\nsourceMatcher: C0000000001\nteamId: T0000000001\nroutingMode: ongoing\nadapterConfig:\n  allow_bot_messages: true\n  trigger: all_messages\n---\n\nTriage CI failures posted to #notifications-cicd.\n";
const LINEAR_LOOP: &str = "---\non: linear\nname: Ready Pool\nsourceMatcher: team-uuid-eng\ntriggerStates: [Ready]\nprojectIds: [proj-launch]\nstartState: In Progress\n---\n\nPick up the Linear ticket and implement it.\n";

async fn count_manual_loops(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM loops WHERE config_source = 'manual'")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test]
async fn sync_creates_then_reconciles_loops(pool: PgPool) {
    let repo = make_repository_row("repo-loops", "user-1");
    compose::seed_repository(&pool, &repo).await.unwrap();
    common::seed_workspace_placing(&pool, "ws-1", &repo.id, "user-1").await;
    // make_repository_row uses https://github.com/test/{id}.git -> org "test".
    insert_github_connection(&pool, "test").await;
    insert_manual_loop(&pool, "tr_manual_1", "manual one", "ws-1")
        .await
        .unwrap();

    let state = test_app_state(pool.clone());
    let auth = auth("user-1");

    let loops = vec![
        parse_loop(GITHUB_LOOP, "java-review").unwrap(),
        parse_loop(SCHEDULE_LOOP, "daily-health-check").unwrap(),
    ];

    let summary = state
        .loop_sync
        .sync_loops(&repo, &auth, loops)
        .await
        .unwrap();
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
        "SELECT t.kind, t.name, t.active, t.config_source, t.source_config_path,
                r.agent_profile, r.prompt, r.run_as_user_id, r.skills,
                aes.adapter_kind, aes.source_matcher \
         FROM loops t \
         JOIN loop_configs r ON r.loop_id = t.id \
         JOIN subscriptions aes ON aes.loop_id = t.id \
         WHERE t.source_repository_id = $1 AND t.source_key = 'java-review' AND t.config_source = 'git'",
    )
    .bind(&repo.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(gh.get::<String, _>("kind"), "adapter_event");
    assert_eq!(
        gh.get::<String, _>("name"),
        "Java Review",
        "the trigger is named by the frontmatter, not by its directory"
    );
    assert_eq!(gh.get::<String, _>("agent_profile"), "claude-code");
    assert_eq!(
        gh.get::<Option<String>, _>("prompt").as_deref(),
        Some("Review this PR.")
    );
    assert_eq!(gh.get::<String, _>("adapter_kind"), "github");
    assert_eq!(gh.get::<String, _>("source_matcher"), "test/repo-loops");
    assert!(!gh.get::<bool, _>("active"), "Git cannot activate a Loop");
    assert_eq!(gh.get::<String, _>("config_source"), "git");
    assert_eq!(
        gh.get::<String, _>("source_config_path"),
        ".moatless/loops/java-review/LOOP.md"
    );
    assert_eq!(
        gh.get::<Option<String>, _>("run_as_user_id"),
        None,
        "Git cannot assign the execution user"
    );
    assert_eq!(
        gh.get::<Vec<String>, _>("skills"),
        vec!["java-review".to_string(), "e2e-testing".to_string()],
        "skill names are stored verbatim — they resolve on the pod, not here"
    );

    // schedule loop -> schedule trigger with cron + message.
    let sched = sqlx::query(
        "SELECT t.kind, r.skills, sts.cron_expression, sts.timezone, sts.message_template \
         FROM loops t \
         JOIN loop_configs r ON r.loop_id = t.id \
         JOIN schedules sts ON sts.loop_id = t.id \
         WHERE t.source_repository_id = $1 AND t.source_key = 'daily-health-check' AND t.config_source = 'git'",
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
    assert!(
        sched.get::<Vec<String>, _>("skills").is_empty(),
        "a LOOP.md with no `skills` key stores an empty list, never NULL"
    );

    assert_eq!(count_manual_loops(&pool).await, 1, "manual Loop untouched");

    // An administrator approves the imported Loop; a later Git snapshot updates
    // its configuration and leaves that approval standing.
    let schedule_id: String = sqlx::query_scalar(
        "SELECT id FROM loops \
         WHERE source_repository_id = $1 AND source_key = 'daily-health-check'",
    )
    .bind(&repo.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let admin = ScopeContext::admin("admin-1");
    assert!(
        activate_loop(&state.loops, &admin, &schedule_id, "approved-user")
            .await
            .unwrap()
    );

    // Re-sync dropping the github_pr loop: it should be removed, schedule kept,
    // manual trigger still untouched.
    let loops = vec![parse_loop(SCHEDULE_LOOP, "daily-health-check").unwrap()];
    let summary = state
        .loop_sync
        .sync_loops(&repo, &auth, loops)
        .await
        .unwrap();
    assert_eq!(summary.created, 0);
    assert_eq!(summary.updated, 1, "schedule loop updated in place");
    assert_eq!(summary.removed, 1, "java-review loop removed");

    let remaining: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM loops WHERE source_repository_id = $1 AND config_source = 'git'",
    )
    .bind(&repo.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(remaining, 1, "only the schedule loop remains");
    let approval = sqlx::query(
        "SELECT t.active, r.run_as_user_id \
         FROM loops t JOIN loop_configs r ON r.loop_id = t.id \
         WHERE t.id = $1",
    )
    .bind(&schedule_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        approval.get::<bool, _>("active"),
        "an accepted re-sync leaves the Loop running"
    );
    assert_eq!(
        approval
            .get::<Option<String>, _>("run_as_user_id")
            .as_deref(),
        Some("approved-user"),
        "Git never touches the run-as identity an administrator selected"
    );
    assert_eq!(
        count_manual_loops(&pool).await,
        1,
        "manual Loop still untouched"
    );
}

#[sqlx::test]
async fn github_pr_loop_without_connection_is_skipped(pool: PgPool) {
    let repo = make_repository_row("repo-noconn", "user-1");
    compose::seed_repository(&pool, &repo).await.unwrap();
    common::seed_workspace_placing(&pool, "ws-1", &repo.id, "user-1").await;
    // No github connection inserted for org "test".

    let state = test_app_state(pool.clone());
    let auth = auth("user-1");

    let loops = vec![parse_loop(GITHUB_LOOP, "java-review").unwrap()];
    let summary = state
        .loop_sync
        .sync_loops(&repo, &auth, loops)
        .await
        .unwrap();

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
        "SELECT count(*) FROM loops WHERE source_repository_id = $1 AND config_source = 'git'",
    )
    .bind(&repo.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 0, "no Loop created for a skipped declaration");
}

#[sqlx::test]
async fn loop_whose_repository_is_placed_nowhere_is_skipped(pool: PgPool) {
    let repo = make_repository_row("repo-unplaced", "user-1");
    compose::seed_repository(&pool, &repo).await.unwrap();
    // No workspace placing the repository, and the declaration names none.

    let state = test_app_state(pool.clone());
    let auth = auth("user-1");

    let loops = vec![parse_loop(SCHEDULE_LOOP, "daily-health-check").unwrap()];
    let summary = state
        .loop_sync
        .sync_loops(&repo, &auth, loops)
        .await
        .unwrap();

    assert_eq!(summary.created, 0);
    assert_eq!(summary.skipped.len(), 1, "unplaced repository skipped");
    assert!(
        summary.skipped[0].reason.contains("no workspace"),
        "reason: {}",
        summary.skipped[0].reason
    );

    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM loops WHERE source_repository_id = $1 AND config_source = 'git'",
    )
    .bind(&repo.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 0, "no Loop created for a skipped declaration");
}

#[sqlx::test]
async fn sync_creates_slack_loop(pool: PgPool) {
    let repo = make_repository_row("repo-slack", "user-1");
    compose::seed_repository(&pool, &repo).await.unwrap();
    common::seed_workspace_placing(&pool, "ws-1", &repo.id, "user-1").await;
    insert_slack_connection(&pool, "T0000000001").await;

    let state = test_app_state(pool.clone());
    let auth = auth("user-1");

    let loops = vec![parse_loop(SLACK_LOOP, "e2e-triage").unwrap()];
    let summary = state
        .loop_sync
        .sync_loops(&repo, &auth, loops)
        .await
        .unwrap();
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
         FROM loops t \
         JOIN loop_configs r ON r.loop_id = t.id \
         JOIN subscriptions aes ON aes.loop_id = t.id \
         WHERE t.source_repository_id = $1 AND t.source_key = 'e2e-triage' AND t.config_source = 'git'",
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
    compose::seed_repository(&pool, &repo).await.unwrap();
    common::seed_workspace_placing(&pool, "ws-1", &repo.id, "user-1").await;
    // Single linear connection — resolved without an explicit teamId.
    insert_linear_connection(&pool, "org-acme").await;

    let state = test_app_state(pool.clone());
    let auth = auth("user-1");

    let loops = vec![parse_loop(LINEAR_LOOP, "ready-pool").unwrap()];
    let summary = state
        .loop_sync
        .sync_loops(&repo, &auth, loops)
        .await
        .unwrap();
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
         FROM loops t \
         JOIN subscriptions aes ON aes.loop_id = t.id \
         WHERE t.source_repository_id = $1 AND t.source_key = 'ready-pool' AND t.config_source = 'git'",
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
    compose::seed_repository(&pool, &repo).await.unwrap();
    common::seed_workspace_placing(&pool, "ws-1", &repo.id, "user-1").await;
    // No linear connection inserted.

    let state = test_app_state(pool.clone());
    let auth = auth("user-1");

    let loops = vec![parse_loop(LINEAR_LOOP, "ready-pool").unwrap()];
    let summary = state
        .loop_sync
        .sync_loops(&repo, &auth, loops)
        .await
        .unwrap();

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
    compose::seed_repository(&pool, &repo).await.unwrap();
    common::seed_workspace_placing(&pool, "ws-1", &repo.id, "user-1").await;
    // No slack connection inserted for team T0000000001.

    let state = test_app_state(pool.clone());
    let auth = auth("user-1");

    let loops = vec![parse_loop(SLACK_LOOP, "e2e-triage").unwrap()];
    let summary = state
        .loop_sync
        .sync_loops(&repo, &auth, loops)
        .await
        .unwrap();

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
        "SELECT count(*) FROM loops WHERE source_repository_id = $1 AND config_source = 'git'",
    )
    .bind(&repo.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 0, "no Loop created for a skipped declaration");
}

/// Re-syncing a managed loop must not wipe `ongoing_task_id` — that column is
/// mutable runtime state set by firing, not by the repo declaration. Regression
/// guard for the delete-then-insert reconcile path in `upsert_managed_loop`.
#[sqlx::test]
async fn resync_preserves_ongoing_task_id(pool: PgPool) {
    let repo = make_repository_row("repo-ongoing", "user-1");
    compose::seed_repository(&pool, &repo).await.unwrap();
    common::seed_workspace_placing(&pool, "ws-1", &repo.id, "user-1").await;
    insert_github_connection(&pool, "test").await;

    let state = test_app_state(pool.clone());
    let auth = auth("user-1");

    let loops = vec![parse_loop(GITHUB_LOOP, "java-review").unwrap()];
    let summary = state
        .loop_sync
        .sync_loops(&repo, &auth, loops)
        .await
        .unwrap();
    assert_eq!(summary.created, 1);

    // Simulate a firing linking an ongoing task to the loop's rule row.
    let loop_id: String = sqlx::query_scalar(
        "SELECT id FROM loops \
         WHERE source_repository_id = $1 AND source_key = 'java-review' AND config_source = 'git'",
    )
    .bind(&repo.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE loop_configs SET ongoing_task_id = 'task-ongoing-1' WHERE loop_id = $1")
        .bind(&loop_id)
        .execute(&pool)
        .await
        .unwrap();

    // Re-sync the same loop; the reconciler must preserve the runtime link.
    let loops = vec![parse_loop(GITHUB_LOOP, "java-review").unwrap()];
    let summary = state
        .loop_sync
        .sync_loops(&repo, &auth, loops)
        .await
        .unwrap();
    assert_eq!(summary.updated, 1, "loop updated in place");

    let ongoing: Option<String> =
        sqlx::query_scalar("SELECT ongoing_task_id FROM loop_configs WHERE loop_id = $1")
            .bind(&loop_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        ongoing.as_deref(),
        Some("task-ongoing-1"),
        "re-sync must preserve the ongoing task link"
    );
}

/// Git owns configuration, Moatless owns execution: a re-sync that rewrites the
/// declaration must leave `active` and the run-as user untouched.
#[sqlx::test]
async fn resync_preserves_activation_across_a_changed_declaration(pool: PgPool) {
    const EDITED_GITHUB_LOOP: &str = "---\non: github_pr\nname: Java Review v2\nagentProfile: codex\ntags: [review, backend]\n---\n\nReview this PR against the backend guidelines.\n";

    let repo = make_repository_row("repo-reapprove", "user-1");
    compose::seed_repository(&pool, &repo).await.unwrap();
    common::seed_workspace_placing(&pool, "ws-1", &repo.id, "user-1").await;
    insert_github_connection(&pool, "test").await;

    let state = test_app_state(pool.clone());
    let auth = auth("user-1");
    state
        .loop_sync
        .sync_loops(
            &repo,
            &auth,
            vec![parse_loop(GITHUB_LOOP, "java-review").unwrap()],
        )
        .await
        .unwrap();
    let loop_id: String = sqlx::query_scalar(
        "SELECT id FROM loops \
         WHERE source_repository_id = $1 AND source_key = 'java-review'",
    )
    .bind(&repo.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let admin = ScopeContext::admin("admin-1");
    activate_loop(&state.loops, &admin, &loop_id, "approved-user")
        .await
        .unwrap();

    let summary = state
        .loop_sync
        .sync_loops(
            &repo,
            &auth,
            vec![parse_loop(EDITED_GITHUB_LOOP, "java-review").unwrap()],
        )
        .await
        .unwrap();
    assert_eq!(summary.updated, 1);

    let row = sqlx::query(
        "SELECT t.name, t.active, r.agent_profile, r.run_as_user_id \
         FROM loops t JOIN loop_configs r ON r.loop_id = t.id \
         WHERE t.id = $1",
    )
    .bind(&loop_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        row.get::<String, _>("name"),
        "Java Review v2",
        "Git configuration still lands"
    );
    assert_eq!(row.get::<String, _>("agent_profile"), "codex");
    assert!(
        row.get::<bool, _>("active"),
        "the Loop keeps running across the edit"
    );
    assert_eq!(
        row.get::<Option<String>, _>("run_as_user_id").as_deref(),
        Some("approved-user"),
        "the administrator's run-as choice is not Git's to change"
    );
}

/// Reclaiming an override is a configuration action too: the row goes back to
/// Git ownership and keeps the execution approval it already had.
#[sqlx::test]
async fn reset_to_git_keeps_the_execution_approval(pool: PgPool) {
    let repo = make_repository_row("repo-reset", "user-1");
    compose::seed_repository(&pool, &repo).await.unwrap();
    common::seed_workspace_placing(&pool, "ws-1", &repo.id, "user-1").await;

    let state = test_app_state(pool.clone());
    let auth = auth("user-1");
    state
        .loop_sync
        .sync_loops(
            &repo,
            &auth,
            vec![parse_loop(SCHEDULE_LOOP, "daily-health-check").unwrap()],
        )
        .await
        .unwrap();
    let loop_id: String = sqlx::query_scalar(
        "SELECT id FROM loops \
         WHERE source_repository_id = $1 AND source_key = 'daily-health-check'",
    )
    .bind(&repo.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let admin = ScopeContext::admin("admin-1");
    activate_loop(&state.loops, &admin, &loop_id, "approved-user")
        .await
        .unwrap();
    state.loops.override_git(&admin, &loop_id).await.unwrap();

    assert!(
        state
            .loops
            .reset_managed_loop(&schedule_upsert(&repo))
            .await
            .unwrap()
    );

    let row = sqlx::query(
        "SELECT t.config_source, t.active, t.name, r.run_as_user_id \
         FROM loops t JOIN loop_configs r ON r.loop_id = t.id \
         WHERE t.id = $1",
    )
    .bind(&loop_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row.get::<String, _>("config_source"), "git");
    assert_eq!(row.get::<String, _>("name"), "Daily Health Check");
    assert!(
        row.get::<bool, _>("active"),
        "reclaiming configuration does not stop an approved Loop"
    );
    assert_eq!(
        row.get::<Option<String>, _>("run_as_user_id").as_deref(),
        Some("approved-user")
    );
}

/// The `SCHEDULE_LOOP` declaration as the reconciler's input shape, for the
/// reset path — which takes an already-fetched Git snapshot rather than
/// parsing.
fn schedule_upsert(repo: &Repository) -> ManagedLoopUpsert {
    ManagedLoopUpsert {
        source_repository_id: repo.id.clone(),
        workspace_id: "ws-1".to_string(),
        source_key: "daily-health-check".to_string(),
        source_config_path: ".moatless/loops/daily-health-check/LOOP.md".to_string(),
        trigger_name: "Daily Health Check".to_string(),
        kind: "schedule".to_string(),
        scope: repo.scope,
        owner_user_id: repo.owner_user_id.clone(),
        created_by: "user-1".to_string(),
        routing_mode: RoutingMode::BySubject,
        prompt: Some("Daily health check.".to_string()),
        tag_ids: Vec::new(),
        memory_preload: Vec::new(),
        agent_properties: Default::default(),
        skills: Vec::new(),
        agent_profile: moatless_types::agent::AgentProfile::ClaudeCode,
        model: None,
        close_after_settled_days: None,
        settle_after_hours: None,
        stop_when_done: None,
        message_buffer_seconds: None,
        room: None,
        fallback_to_other_agent: true,
        adapter: None,
        schedule: Some(NewScheduleSource {
            cron_expression: "0 0 6 * * MON-FRI *".to_string(),
            timezone: "Europe/Stockholm".to_string(),
            message_template: "Run the daily check.".to_string(),
            next_fire_at: Utc::now() + chrono::Duration::hours(1),
        }),
    }
}

#[sqlx::test]
async fn resync_flags_an_override_whose_directory_is_gone(pool: PgPool) {
    let repo = make_repository_row("repo-override-gone", "user-1");
    compose::seed_repository(&pool, &repo).await.unwrap();
    common::seed_workspace_placing(&pool, "ws-1", &repo.id, "user-1").await;
    insert_github_connection(&pool, "test").await;

    let state = test_app_state(pool.clone());
    let auth = auth("user-1");
    let loops = vec![parse_loop(GITHUB_LOOP, "java-review").unwrap()];
    state
        .loop_sync
        .sync_loops(&repo, &auth, loops)
        .await
        .unwrap();
    let loop_id: String = sqlx::query_scalar(
        "SELECT id FROM loops \
         WHERE source_repository_id = $1 AND source_key = 'java-review'",
    )
    .bind(&repo.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let admin = ScopeContext::admin("admin-1");
    state.loops.override_git(&admin, &loop_id).await.unwrap();

    let summary = state
        .loop_sync
        .sync_loops(&repo, &auth, Vec::new())
        .await
        .unwrap();
    assert_eq!(summary.removed, 0);
    assert_eq!(summary.skipped.len(), 1);
    assert_eq!(summary.skipped[0].loop_name, "java-review");
    assert!(
        summary.skipped[0]
            .reason
            .contains("no longer exists in Git")
    );
}

#[sqlx::test]
async fn resync_skips_an_override_and_preserves_its_approval(pool: PgPool) {
    let repo = make_repository_row("repo-override", "user-1");
    compose::seed_repository(&pool, &repo).await.unwrap();
    common::seed_workspace_placing(&pool, "ws-1", &repo.id, "user-1").await;
    insert_github_connection(&pool, "test").await;

    let state = test_app_state(pool.clone());
    let auth = auth("user-1");
    let loops = vec![parse_loop(GITHUB_LOOP, "java-review").unwrap()];
    state
        .loop_sync
        .sync_loops(&repo, &auth, loops)
        .await
        .unwrap();

    let loop_id: String = sqlx::query_scalar(
        "SELECT id FROM loops \
         WHERE source_repository_id = $1 AND source_key = 'java-review'",
    )
    .bind(&repo.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let admin = ScopeContext::admin("admin-1");
    activate_loop(&state.loops, &admin, &loop_id, "approved-user")
        .await
        .unwrap();
    state.loops.override_git(&admin, &loop_id).await.unwrap();

    let loops = vec![parse_loop(GITHUB_LOOP, "java-review").unwrap()];
    let summary = state
        .loop_sync
        .sync_loops(&repo, &auth, loops)
        .await
        .unwrap();
    assert_eq!(summary.updated, 0);
    assert_eq!(summary.overrides_skipped, 1);
    assert_eq!(summary.skipped.len(), 1);
    assert!(summary.skipped[0].reason.contains("local override"));

    let row = sqlx::query(
        "SELECT t.config_source, t.active, r.run_as_user_id \
         FROM loops t JOIN loop_configs r ON r.loop_id = t.id \
         WHERE t.id = $1",
    )
    .bind(&loop_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row.get::<String, _>("config_source"), "manual");
    assert!(row.get::<bool, _>("active"));
    assert_eq!(
        row.get::<Option<String>, _>("run_as_user_id").as_deref(),
        Some("approved-user")
    );
}

#[sqlx::test]
async fn concurrent_activation_and_resync_finish_in_a_serial_state(pool: PgPool) {
    let repo = make_repository_row("repo-concurrent", "user-1");
    compose::seed_repository(&pool, &repo).await.unwrap();
    common::seed_workspace_placing(&pool, "ws-1", &repo.id, "user-1").await;
    insert_github_connection(&pool, "test").await;

    let state = test_app_state(pool.clone());
    let auth = auth("user-1");
    state
        .loop_sync
        .sync_loops(
            &repo,
            &auth,
            vec![parse_loop(GITHUB_LOOP, "java-review").unwrap()],
        )
        .await
        .unwrap();
    let loop_id: String = sqlx::query_scalar(
        "SELECT id FROM loops \
         WHERE source_repository_id = $1 AND source_key = 'java-review'",
    )
    .bind(&repo.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let admin = ScopeContext::admin("admin-1");

    let activate = activate_loop(&state.loops, &admin, &loop_id, "approved-user");
    let resync = state.loop_sync.sync_loops(
        &repo,
        &auth,
        vec![parse_loop(GITHUB_LOOP, "java-review").unwrap()],
    );
    let (activate_result, sync_result) = tokio::join!(activate, resync);
    assert!(activate_result.unwrap());
    sync_result.unwrap();

    let row = sqlx::query(
        "SELECT t.active, r.run_as_user_id \
         FROM loops t JOIN loop_configs r ON r.loop_id = t.id \
         WHERE t.id = $1",
    )
    .bind(&loop_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let active = row.get::<bool, _>("active");
    let run_as_user_id = row.get::<Option<String>, _>("run_as_user_id");
    assert_eq!(
        active,
        run_as_user_id.is_some(),
        "the final row must equal activation-last or sync-last, never a half-approved merge"
    );
}

const ORG_TRIGGER_LOOP: &str = "---\non: github_pr\nname: gate-review\nsourceMatcher: test/*\nworkspace: auto\nskills: [pr-review]\nadapterConfig:\n  create_on: check_run\n  check_run:\n    names: [soap-review-request]\n    app_ids: [5421124]\n---\n\nReview this PR.\n";
const ORG_WILDCARD_WITHOUT_CREATE_ON: &str =
    "---\non: github_pr\nname: org-review\nsourceMatcher: test/*\n---\n\nReview this PR.\n";
const AUTO_WORKSPACE_WITHOUT_CREATE_ON: &str =
    "---\non: github_pr\nname: auto-review\nworkspace: auto\n---\n\nReview this PR.\n";

#[sqlx::test]
async fn an_org_trigger_loop_syncs_only_with_create_on_check_run(pool: PgPool) {
    let repo = make_repository_row("repo-org-review", "user-1");
    compose::seed_repository(&pool, &repo).await.unwrap();
    common::seed_workspace_placing(&pool, "ws-1", &repo.id, "user-1").await;
    insert_github_connection(&pool, "test").await;
    let state = test_app_state(pool.clone());

    let summary = state
        .loop_sync
        .sync_loops(
            &repo,
            &auth("user-1"),
            vec![
                parse_loop(ORG_TRIGGER_LOOP, "gate-review").unwrap(),
                parse_loop(ORG_WILDCARD_WITHOUT_CREATE_ON, "org-review").unwrap(),
                parse_loop(AUTO_WORKSPACE_WITHOUT_CREATE_ON, "auto-review").unwrap(),
            ],
        )
        .await
        .unwrap();

    assert_eq!(summary.created, 1, "skipped: {:?}", summary.skipped);
    let mut reasons: Vec<&str> = summary
        .skipped
        .iter()
        .map(|skip| skip.reason.as_str())
        .collect();
    reasons.sort_unstable();
    assert_eq!(reasons.len(), 2);
    assert!(
        reasons
            .iter()
            .all(|reason| reason.contains("create_on: check_run")),
        "{reasons:?}"
    );
    let (source_matcher, workspace_id): (String, String) = sqlx::query_as(
        "SELECT s.source_matcher, c.workspace_id FROM loops l \
         JOIN subscriptions s ON s.loop_id = l.id \
         JOIN loop_configs c ON c.loop_id = l.id \
         WHERE l.name = 'gate-review'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(source_matcher, "test/*");
    assert_eq!(workspace_id, "ws-1");
}

type ScheduleState = (
    Option<chrono::DateTime<Utc>>,
    chrono::DateTime<Utc>,
    Option<String>,
);

/// `(last_fired_at, next_fire_at, locked_by)` of the schedule synced from
/// `daily-health-check`.
async fn schedule_state(pool: &PgPool, repo: &Repository) -> ScheduleState {
    sqlx::query_as(
        "SELECT s.last_fired_at, s.next_fire_at, s.locked_by FROM schedules s \
         JOIN loops t ON t.id = s.loop_id \
         WHERE t.source_repository_id = $1 AND t.source_key = 'daily-health-check'",
    )
    .bind(&repo.id)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// A schedule's firing state is runtime state: re-syncing an unchanged
/// declaration keeps it, and only a cron change recomputes `next_fire_at`.
#[sqlx::test]
async fn resync_keeps_schedule_firing_state_until_the_cron_changes(pool: PgPool) {
    const HOURLY_LOOP: &str = "---\non: schedule\nname: Daily Health Check\nschedule:\n  cron: \"0 0 * * * * *\"\n  timezone: Europe/Stockholm\n  message: \"Run the daily check.\"\n---\n\nDaily health check.\n";
    let repo = make_repository_row("repo-schedule-state", "user-1");
    compose::seed_repository(&pool, &repo).await.unwrap();
    common::seed_workspace_placing(&pool, "ws-1", &repo.id, "user-1").await;
    let state = test_app_state(pool.clone());
    let auth = auth("user-1");
    let sync = |content: &'static str| {
        let state = state.clone();
        let repo = repo.clone();
        let auth = auth.clone();
        async move {
            let loops = vec![parse_loop(content, "daily-health-check").unwrap()];
            state
                .loop_sync
                .sync_loops(&repo, &auth, loops)
                .await
                .unwrap()
        }
    };
    sync(SCHEDULE_LOOP).await;

    // A firing recorded its time and moved the next fire far out; a worker
    // holds the lock.
    let fired = Utc::now() - chrono::Duration::hours(2);
    let next = Utc::now() + chrono::Duration::days(30);
    sqlx::query(
        "UPDATE schedules SET last_fired_at = $2, next_fire_at = $3, locked_by = 'worker-1', \
         locked_until = now() + interval '2 minutes' \
         WHERE loop_id = (SELECT id FROM loops WHERE source_repository_id = $1)",
    )
    .bind(&repo.id)
    .bind(fired)
    .bind(next)
    .execute(&pool)
    .await
    .unwrap();

    sync(SCHEDULE_LOOP).await;
    let (last_fired_at, next_fire_at, locked_by) = schedule_state(&pool, &repo).await;
    assert_eq!(
        last_fired_at.map(|t| t.timestamp_micros()),
        Some(fired.timestamp_micros())
    );
    assert_eq!(next_fire_at.timestamp_micros(), next.timestamp_micros());
    assert_eq!(locked_by.as_deref(), Some("worker-1"));

    sync(HOURLY_LOOP).await;
    let (last_fired_at, next_fire_at, _) = schedule_state(&pool, &repo).await;
    assert_eq!(
        last_fired_at.map(|t| t.timestamp_micros()),
        Some(fired.timestamp_micros())
    );
    assert!(
        next_fire_at <= Utc::now() + chrono::Duration::hours(1),
        "a changed cron recomputes the next fire: {next_fire_at}"
    );
}
