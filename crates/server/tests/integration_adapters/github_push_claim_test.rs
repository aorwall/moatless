#![cfg(feature = "db-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr
)]
//! A Task's own `git push` must not come back to it as a notification.
//!
//! A push bypasses the backend, so the `pull_request.synchronize` webhook that
//! follows carries no `Moatless task:` footer and no typed route naming the
//! Task. The sandbox files a claim on the sha instead, keyed by repository and
//! sha (no PR number exists yet); routing later has the full `synchronize` id,
//! number included. The alias is what joins the two, asserted end to end here.

use std::sync::Arc;

use crate::common;

use moatless_adapter_model::capabilities::AdapterInboundPresentation;
use moatless_ports::task::EventSubject;
use moatless_tasks::task::test_support as task_fixtures;
use moatless_workspaces::compose;
use sqlx::PgPool;

use common::{make_repository_row, make_task_row};

const SHA: &str = "be1d882e6f563d141c849478c1429494fc435890";

/// The label `task_outbound_events.provider` holds for GitHub PR rows, which is
/// also what routing asks under.
const ADAPTER: &str = "github_pr";

async fn seed_task(pool: &PgPool, task_id: &str, user: &str) {
    let repo = make_repository_row(&format!("repo-{task_id}"), user);
    compose::seed_repository(pool, &repo)
        .await
        .expect("seed repo");
    let mut task = make_task_row(task_id, user);
    task.repository_id = Some(repo.id.clone());
    let workspace_id = format!("ws_{}", repo.id);
    common::seed_workspace(pool, &workspace_id, user).await;
    task.workspace_id = Some(workspace_id);
    task_fixtures::seed_task(pool, &task)
        .await
        .expect("seed task");
}

/// Build the adapter for its external-id codec alone. Routing reaches the same
/// function through `RoutingService::external_id_aliases`.
fn adapter() -> moatless_adapter_github_pr::adapter::GithubPrAdapter {
    moatless_adapter_github_pr::adapter::GithubPrAdapter::new(
        None,
        "https://app.example.com".to_string(),
        moatless_core::config::DEFAULT_GITHUB_API_BASE.to_string(),
        moatless_identity::git_host_tokens::seam_over(Arc::new(
            moatless_identity::auth::test_support::StubProviderTokens::default(),
        )),
        moatless_core::config::HttpConfig::default().github,
    )
}

/// The whole point: a sha claimed with no pull request number is found again by
/// the `synchronize` id that carries one.
#[sqlx::test]
async fn a_pushed_sha_names_its_task_when_the_synchronize_event_arrives(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;
    let transcript = moatless_tasks::message::test_support::transcript(&pool);

    moatless_adapter_github_pr::push_claim::record(
        transcript.as_ref(),
        "task-1",
        "soaplabs",
        "soap-knowledge",
        SHA,
    )
    .await
    .unwrap();

    let ids = adapter().external_id_aliases(&format!(
        "github:pr:synchronize:soaplabs/soap-knowledge#101:{SHA}"
    ));

    assert_eq!(
        moatless_tasks::message::test_support::task_id_for_external_event_on_subject(
            &pool,
            &ids,
            EventSubject {
                adapter: ADAPTER,
                source_name: "soaplabs/soap-knowledge",
                subject_key: "soaplabs/soap-knowledge#101",
            },
        )
        .await
        .unwrap()
        .as_deref(),
        Some("task-1"),
    );
}

/// An unclaimed sha names no Task, so another's push is delivered — guarding
/// against a fetch being wrongly claimed as a push, which would misattribute a
/// collaborator's commits as the Task's work.
#[sqlx::test]
async fn an_unclaimed_sha_names_no_task(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;
    let ids = adapter().external_id_aliases(&format!(
        "github:pr:synchronize:soaplabs/soap-knowledge#101:{SHA}"
    ));

    assert_eq!(
        moatless_tasks::message::test_support::task_id_for_external_event_on_subject(
            &pool,
            &ids,
            EventSubject {
                adapter: ADAPTER,
                source_name: "soaplabs/soap-knowledge",
                subject_key: "soaplabs/soap-knowledge#101",
            },
        )
        .await
        .unwrap(),
        None,
    );
}

/// The claim carries no pull request number, so the repository is the only
/// thing keeping it apart from another repository's push of the same sha. A sha
/// collides across repositories whenever one is forked from the other.
#[sqlx::test]
async fn a_claim_answers_only_for_the_repository_it_was_filed_in(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;
    let transcript = moatless_tasks::message::test_support::transcript(&pool);

    moatless_adapter_github_pr::push_claim::record(
        transcript.as_ref(),
        "task-1",
        "soaplabs",
        "soap-knowledge",
        SHA,
    )
    .await
    .unwrap();

    let ids = adapter().external_id_aliases(&format!(
        "github:pr:synchronize:acme/soap-knowledge#101:{SHA}"
    ));

    assert_eq!(
        moatless_tasks::message::test_support::task_id_for_external_event_on_subject(
            &pool,
            &ids,
            EventSubject {
                adapter: ADAPTER,
                source_name: "acme/soap-knowledge",
                subject_key: "acme/soap-knowledge#101",
            },
        )
        .await
        .unwrap(),
        None,
    );
}
