//! The search against a real database: what it matches, in which order, and
//! what access and status hide.

use moatless_types::access_control::ScopeContext;
use moatless_types::task::TaskStatus;
use moatless_workspaces::test_support::make_repository_row;
use sqlx::PgPool;

use super::TaskSearch;
use crate::task::test_support::{make_task_row, seed_task, unfollow_task};

const USER_1: &str = "11111111-1111-1111-1111-111111111111";
const USER_2: &str = "22222222-2222-2222-2222-222222222222";

/// Seed one open Task owned by `user_id`, who follows it as its creator.
async fn seed_owned(pool: &PgPool, task_id: &str, user_id: &str) {
    seed_task(pool, &make_task_row(task_id, user_id))
        .await
        .expect("seed task");
}

async fn seed_repo(pool: &PgPool) {
    let repo = make_repository_row("repo-1", USER_1);
    moatless_workspaces::compose::seed_repository(pool, &repo)
        .await
        .expect("seed repo");
}

/// One Message whose text is its inbox item's, as a person's or a sandbox
/// wake's is stored.
async fn seed_inbox_message(pool: &PgPool, task_id: &str, position: i64, origin: &str, text: &str) {
    let inbox = uuid::Uuid::new_v4();
    let sent_by = (origin == "user").then_some(USER_1);
    sqlx::query(
        "INSERT INTO task_inbox (id, task_id, text, origin, sent_by, status, delivered_at)
         VALUES ($1, $2, $3, $4, $5, 'delivered', now())",
    )
    .bind(inbox)
    .bind(task_id)
    .bind(text)
    .bind(origin)
    .bind(sent_by)
    .execute(pool)
    .await
    .expect("seed inbox item");
    sqlx::query(
        "INSERT INTO conversation_messages (uuid, task_id, position, author, kind, inbox_id)
         VALUES ($1, $2, $3, 'user', 'text', $4)",
    )
    .bind(uuid::Uuid::new_v4())
    .bind(task_id)
    .bind(position)
    .bind(inbox)
    .execute(pool)
    .await
    .expect("seed person's message");
}

/// One agent Message, frozen so it needs no trajectory event behind it.
async fn seed_agent_message(pool: &PgPool, task_id: &str, position: i64, text: &str) {
    sqlx::query(
        "INSERT INTO conversation_messages
             (uuid, task_id, position, author, kind, text, frozen, legacy)
         VALUES ($1, $2, $3, 'agent', 'text', $4, true, '{}')",
    )
    .bind(uuid::Uuid::new_v4())
    .bind(task_id)
    .bind(position)
    .bind(text)
    .execute(pool)
    .await
    .expect("seed agent message");
}

/// Every open Task the viewer may read is searched, followed or not. A
/// person's Message outranks the agent's, which outranks a Task name; a
/// sandbox wake, another user's private Task and a closed Task match nothing;
/// `%` is a literal.
#[sqlx::test]
async fn a_search_reaches_every_readable_open_task_and_ranks_by_source(pool: PgPool) {
    seed_repo(&pool).await;
    seed_owned(&pool, "by-person", USER_1).await;
    seed_inbox_message(&pool, "by-person", 1, "user", "deploy 100% now").await;
    seed_owned(&pool, "by-agent", USER_1).await;
    seed_agent_message(&pool, "by-agent", 1, "deploy 100% done").await;
    seed_owned(&pool, "by-wake", USER_1).await;
    seed_inbox_message(&pool, "by-wake", 1, "sandbox", "deploy 100% woke").await;
    seed_owned(&pool, "unfollowed", USER_1).await;
    unfollow_task(&pool, USER_1, "unfollowed")
        .await
        .expect("drop the creator's follow");
    seed_agent_message(&pool, "unfollowed", 1, "deploy 100% elsewhere").await;
    seed_owned(&pool, "theirs", USER_2).await;
    seed_inbox_message(&pool, "theirs", 1, "user", "deploy 100% private").await;
    seed_owned(&pool, "percent-only", USER_1).await;
    seed_agent_message(&pool, "percent-only", 1, "deploy 1000 rows").await;
    let mut closed = make_task_row("closed", USER_1);
    closed.status = TaskStatus::Closed;
    seed_task(&pool, &closed).await.expect("seed task");
    seed_inbox_message(&pool, "closed", 1, "user", "deploy 100% closed").await;

    // Pinned rather than left to insert order, which can tie.
    for (task_id, minutes_ago) in [("by-person", 3), ("by-agent", 2), ("unfollowed", 1)] {
        sqlx::query(
            "UPDATE tasks SET updated_at = now() - make_interval(mins => $2) WHERE task_id = $1",
        )
        .bind(task_id)
        .bind(minutes_ago)
        .execute(&pool)
        .await
        .expect("pin updated_at");
    }

    let search = TaskSearch::new(pool.clone());
    let viewer = ScopeContext::user(USER_1.to_string());

    let matches = search.search(&viewer, "100%", 10).await.unwrap();
    let found: Vec<(&str, bool)> = matches
        .iter()
        .map(|m| (m.task_id.as_str(), m.from_user))
        .collect();
    assert_eq!(
        found,
        // Within a rank, the most recently updated Task first.
        vec![
            ("by-person", true),
            ("unfollowed", false),
            ("by-agent", false),
        ]
    );

    let by_name = search.search(&viewer, "task by-wake", 10).await.unwrap();
    assert_eq!(by_name.len(), 1);
    assert_eq!(by_name[0].matched_text, "Test task by-wake");
    assert!(by_name[0].matched_at.is_none());
}

/// Any `limit` answers a prefix of the full ranking, and a Task answers with
/// its best source's newest match: a person's Message over the agent's over
/// the name, however recently the lower-ranked Task was updated.
#[sqlx::test]
async fn a_limited_search_is_a_prefix_of_the_full_ranking(pool: PgPool) {
    seed_repo(&pool).await;
    seed_owned(&pool, "person-twice", USER_1).await;
    seed_inbox_message(&pool, "person-twice", 1, "user", "alpha older").await;
    seed_agent_message(&pool, "person-twice", 2, "alpha from the agent").await;
    seed_inbox_message(&pool, "person-twice", 3, "user", "alpha newer").await;
    seed_owned(&pool, "person-once", USER_1).await;
    seed_inbox_message(&pool, "person-once", 1, "user", "alpha once").await;
    seed_owned(&pool, "alpha-agent", USER_1).await;
    seed_agent_message(&pool, "alpha-agent", 1, "alpha first").await;
    seed_agent_message(&pool, "alpha-agent", 2, "alpha second").await;
    seed_owned(&pool, "alpha-name-only", USER_1).await;
    seed_inbox_message(&pool, "alpha-name-only", 1, "sandbox", "alpha woke").await;

    for (task_id, minutes_ago) in [
        ("alpha-name-only", 1),
        ("alpha-agent", 2),
        ("person-once", 3),
        ("person-twice", 4),
    ] {
        sqlx::query(
            "UPDATE tasks SET updated_at = now() - make_interval(mins => $2) WHERE task_id = $1",
        )
        .bind(task_id)
        .bind(minutes_ago)
        .execute(&pool)
        .await
        .expect("pin updated_at");
    }

    let search = TaskSearch::new(pool.clone());
    let viewer = ScopeContext::user(USER_1.to_string());
    let answer = |limit| {
        let search = search.clone();
        let viewer = viewer.clone();
        async move {
            search
                .search(&viewer, "alpha", limit)
                .await
                .unwrap()
                .into_iter()
                .map(|m| (m.task_id, m.from_user, m.matched_text))
                .collect::<Vec<_>>()
        }
    };

    let full = answer(10).await;
    let expected: Vec<(String, bool, String)> = [
        ("person-once", true, "alpha once"),
        ("person-twice", true, "alpha newer"),
        ("alpha-agent", false, "alpha second"),
        ("alpha-name-only", true, "Test task alpha-name-only"),
    ]
    .into_iter()
    .map(|(task_id, from_user, text)| (task_id.to_string(), from_user, text.to_string()))
    .collect();
    assert_eq!(full, expected);
    for limit in 1..=3 {
        assert_eq!(
            answer(limit).await,
            full[..usize::try_from(limit).unwrap()],
            "limit {limit}"
        );
    }
}
