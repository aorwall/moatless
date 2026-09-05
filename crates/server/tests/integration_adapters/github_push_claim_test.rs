#![cfg(feature = "db-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr
)]
//! A Task's own `git push` must not come back to it as a notification.
//!
//! A push reaches GitHub without passing through the backend, so the
//! `pull_request.synchronize` webhook that follows carries nothing naming the
//! Task behind it: the payload has no body to hold the `Moatless task:` footer,
//! and no typed route recorded an id. The sandbox files a claim on the sha as
//! the push completes instead.
//!
//! Two halves have to meet for that claim to be found, and they are written on
//! opposite sides of the push. The sandbox knows the repository and the sha and
//! not the pull request number, which on a first push does not exist yet.
//! Routing knows the whole `synchronize` id, number included. What joins them
//! is the alias, and this is where it is asserted end to end.

use crate::common;

use std::sync::Arc;

use moatless_adapter_model::capabilities::AdapterInboundPresentation;
use moatless_ports::task::EventSubject;
use moatless_tasks::message::dao::MessageDao;
use moatless_tasks::message::dao::PgMessageDao;
use moatless_tasks::task::dao as task_dao;
use moatless_workspaces::repository::dao as repository_dao;
use sqlx::PgPool;

use common::{make_repository_row, make_task_row};

const SHA: &str = "be1d882e6f563d141c849478c1429494fc435890";

/// The label `task_outbound_events.provider` holds for GitHub PR rows, which is
/// also what routing asks under.
const ADAPTER: &str = "github_pr";

async fn seed_task(pool: &PgPool, task_id: &str, user: &str) {
    let repo = make_repository_row(&format!("repo-{task_id}"), user);
    repository_dao::insert(pool, &repo)
        .await
        .expect("seed repo");
    let task = make_task_row(task_id, user);
    task_dao::insert_task(pool, &task).await.expect("seed task");
}

/// Build the adapter for its external-id codec alone. Routing reaches the same
/// function through `RoutingService::external_id_aliases`.
fn adapter(pool: &PgPool) -> moatless_adapter_github_pr::adapter::GithubPrAdapter {
    moatless_adapter_github_pr::adapter::GithubPrAdapter::new(
        None,
        "https://app.example.com".to_string(),
        moatless_identity::git_host_tokens::seam_over(Arc::new(
            moatless_identity::auth::provider_token_dao::PgProviderTokenDao(pool.clone()),
        )),
    )
}

/// The whole point: a sha claimed with no pull request number is found again by
/// the `synchronize` id that carries one.
#[sqlx::test]
async fn a_pushed_sha_names_its_task_when_the_synchronize_event_arrives(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;
    let messages = Arc::new(PgMessageDao(pool.clone()));

    moatless_adapter_github_pr::push_claim::record(
        &moatless_tasks::ports::Transcript::new(messages.clone()),
        "task-1",
        "soaplabs",
        "soap-knowledge",
        SHA,
    )
    .await
    .unwrap();

    let ids = adapter(&pool).external_id_aliases(&format!(
        "github:pr:synchronize:soaplabs/soap-knowledge#101:{SHA}"
    ));

    assert_eq!(
        messages
            .find_task_id_by_any_external_event_id_on_subject(
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

/// A sha nothing claimed names no Task, so a push by anyone else is delivered.
/// This is the case reading the reflog message protects: were a fetch claimed as
/// a push, the collaborator's commits would arrive here as the Task's own work
/// and the notification would be dropped.
#[sqlx::test]
async fn an_unclaimed_sha_names_no_task(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;
    let messages = PgMessageDao(pool.clone());

    let ids = adapter(&pool).external_id_aliases(&format!(
        "github:pr:synchronize:soaplabs/soap-knowledge#101:{SHA}"
    ));

    assert_eq!(
        messages
            .find_task_id_by_any_external_event_id_on_subject(
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

/// The claim carries no pull request number, so the repository is the only thing
/// keeping it apart from another repository's push of the same sha. A sha
/// collides across repositories whenever one is forked from the other.
#[sqlx::test]
async fn a_claim_answers_only_for_the_repository_it_was_filed_in(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;
    let messages = Arc::new(PgMessageDao(pool.clone()));

    moatless_adapter_github_pr::push_claim::record(
        &moatless_tasks::ports::Transcript::new(messages.clone()),
        "task-1",
        "soaplabs",
        "soap-knowledge",
        SHA,
    )
    .await
    .unwrap();

    let ids = adapter(&pool).external_id_aliases(&format!(
        "github:pr:synchronize:acme/soap-knowledge#101:{SHA}"
    ));

    assert_eq!(
        messages
            .find_task_id_by_any_external_event_id_on_subject(
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
