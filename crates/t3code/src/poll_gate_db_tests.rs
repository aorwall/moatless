//! A reconnect wave against a small pool: the shell subscriptions it opens take
//! at most [`PollGate::for_pool`]'s share of connections, so a worker query and
//! the readiness probe still get one.

use std::time::Duration;

use moatless_core::db::{create_probe_pool, probe};
use moatless_tasks::task::test_support::make_task_row;
use moatless_types::access_control::ScopeContext;
use moatless_workspaces::test_support::make_repository_row;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use tokio::task::JoinSet;

use crate::compose::t3_graph;
use crate::config::AgentModelDefaults;
use crate::subscriptions::shell_subscription;

const USER: &str = "11111111-1111-1111-1111-111111111111";
const WAVE: usize = 32;
const POOL: u32 = 4;

async fn seed_followed_task(pool: &PgPool) {
    let repo = make_repository_row("repo-1", USER);
    moatless_workspaces::compose::seed_repository(pool, &repo)
        .await
        .expect("seed repo");
    let task = make_task_row("task-wave", USER);
    moatless_tasks::task::test_support::seed_task(pool, &task)
        .await
        .expect("seed task");
    moatless_tasks::task::test_support::follow_task(pool, USER, "task-wave")
        .await
        .expect("follow task");
}

#[sqlx::test]
async fn a_reconnect_wave_leaves_connections_for_workers_and_readiness(pool: PgPool) {
    seed_followed_task(&pool).await;
    let small = PgPoolOptions::new()
        .max_connections(POOL)
        .min_connections(POOL)
        .acquire_timeout(Duration::from_secs(5))
        .connect_with((*pool.connect_options()).clone())
        .await
        .expect("small pool");
    let reads = t3_graph(
        small.clone(),
        &moatless_tasks::compose::task_graph(small.clone()),
    )
    .reads()
    .clone();
    let environment = reads.listing_head().await.expect("listing head").identity;
    let readiness = create_probe_pool(&small);
    // Warm, as on a running replica the probe has already answered.
    probe(&readiness).await.expect("probe connects");
    let (release, released) = tokio::sync::watch::channel(false);

    let mut wave = JoinSet::new();
    for _ in 0..WAVE {
        let (reads, small, environment) = (reads.clone(), small.clone(), environment.clone());
        let mut released = released.clone();
        wave.spawn(async move {
            let open = async {
                let opened = shell_subscription(
                    &reads,
                    &AgentModelDefaults::default(),
                    &ScopeContext::user(USER.to_string()),
                    &environment,
                    None,
                )
                .await;
                // A slow snapshot build: hold a connection until released.
                let held = small.acquire().await;
                let _ = released.wait_for(|done| *done).await;
                drop(held);
                opened.map(|opened| opened.values)
            };
            reads.poll_gate().run(open).await
        });
    }
    let in_use = || (small.size() as usize).saturating_sub(small.num_idle());
    let saturated = async {
        while in_use() < (POOL / 2) as usize {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    tokio::time::timeout(Duration::from_secs(5), saturated)
        .await
        .expect("the wave holds the gate's share of the pool");
    // Settle: the shell listing admits one snapshot at a time, so an ungated
    // wave reaches the pool one subscription after another.
    tokio::time::sleep(Duration::from_secs(1)).await;
    let wave_held = in_use();

    let worker = sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(&small);
    // Without the gate the wave holds every connection until released, so this
    // waits out its budget.
    let worker = tokio::time::timeout(Duration::from_secs(2), worker).await;
    let ready = tokio::time::timeout(Duration::from_millis(500), probe(&readiness)).await;
    let _ = release.send(true);
    let opened = wave.join_all().await;

    assert_eq!(wave_held, (POOL / 2) as usize, "connections the wave held");
    assert!(matches!(worker, Ok(Ok(1))), "worker query: {worker:?}");
    assert!(matches!(ready, Ok(Ok(1))), "readiness probe: {ready:?}");
    assert_eq!(opened.len(), WAVE);
    for values in opened {
        let values = values.expect("every subscription in the wave opens");
        assert_eq!(values[0]["kind"], "snapshot");
    }
}
