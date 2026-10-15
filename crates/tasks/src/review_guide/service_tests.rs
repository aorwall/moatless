use std::sync::Arc;

use chrono::Utc;
use moatless_types::access_control::ScopeContext;

use super::dao::MockReviewGuideDao;
use super::error::ReviewGuideError;
use super::models::{RUN_RUNNING, ReviewGuideRun};
use super::operations::ReviewGuides;
use super::schemas::{
    ReviewGuideChange, ReviewGuideExcerpt, ReviewGuideFileHunks, ReviewGuideLineRange,
    ReviewGuideSide, ReviewGuideStop, SubmitReviewGuideRequest,
};
use super::service::{run_for_task, submit};
use crate::task::dao::MockTaskDao;
use crate::task::operations::TaskCommands;
use crate::task::test_support::make_task_row;

fn scope() -> ScopeContext {
    ScopeContext::user("user-1")
}

/// The caller owns `task-1` and can read, but not change, `task-shared`.
fn tasks() -> TaskCommands {
    let mut store = MockTaskDao::new();
    store
        .expect_find_by_id()
        .returning(|task_id, _| match task_id {
            "task-1" => Ok(Some(make_task_row(task_id, "user-1"))),
            "task-shared" => Ok(Some(make_task_row(task_id, "user-2"))),
            _ => Ok(None),
        });
    TaskCommands::new(Arc::new(store))
}

fn run() -> ReviewGuideRun {
    ReviewGuideRun {
        host: "github.com".into(),
        repository: "acme/app".into(),
        number: 7,
        run_id: "run-1".into(),
        head_sha: "head".into(),
        base_sha: "base".into(),
        task_id: Some("task-1".into()),
        status: RUN_RUNNING.into(),
        detail: None,
        started_at: Utc::now(),
        failed_at: None,
    }
}

fn store_running(running: Option<ReviewGuideRun>) -> MockReviewGuideDao {
    let mut store = MockReviewGuideDao::new();
    store
        .expect_running_run_for_task()
        .returning(move |_| Ok(running.clone()));
    store
}

fn request(start: u32, end: u32) -> SubmitReviewGuideRequest {
    let mut change = ReviewGuideChange {
        files: vec!["src/a.rs".into()],
        ..Default::default()
    };
    change.hunks.insert(
        "src/a.rs".into(),
        ReviewGuideFileHunks {
            new: vec![ReviewGuideLineRange { start: 1, end: 10 }],
            old: vec![],
        },
    );
    SubmitReviewGuideRequest {
        stops: vec![ReviewGuideStop {
            title: "The change".into(),
            summary: "What it does.".into(),
            attention: None,
            excerpts: vec![ReviewGuideExcerpt {
                path: "src/a.rs".into(),
                side: ReviewGuideSide::New,
                start,
                end,
                caption: String::new(),
            }],
            files: vec![],
            notes: vec![],
            questions: vec![],
        }],
        change,
    }
}

#[tokio::test]
async fn run_for_task_reports_the_range_to_describe() {
    let guides = ReviewGuides::new(Arc::new(store_running(Some(run()))));
    let response = run_for_task(&tasks(), &guides, &scope(), "task-1")
        .await
        .unwrap();
    assert_eq!(response.repository, "acme/app");
    assert_eq!(response.number, 7);
    assert_eq!(
        (response.base_sha.as_str(), response.head_sha.as_str()),
        ("base", "head")
    );
}

#[tokio::test]
async fn a_task_the_caller_cannot_change_is_refused() {
    let guides = ReviewGuides::new(Arc::new(MockReviewGuideDao::new()));
    let err = run_for_task(&tasks(), &guides, &scope(), "task-shared")
        .await
        .unwrap_err();
    assert!(matches!(err, ReviewGuideError::Forbidden(_)), "{err:?}");
}

#[tokio::test]
async fn a_task_without_a_running_run_cannot_submit() {
    let guides = ReviewGuides::new(Arc::new(store_running(None)));
    let err = submit(&tasks(), &guides, &scope(), "task-1", request(2, 4))
        .await
        .unwrap_err();
    assert!(matches!(err, ReviewGuideError::NoRun(_)), "{err:?}");
}

#[tokio::test]
async fn a_guide_whose_excerpts_all_miss_the_change_is_refused() {
    let guides = ReviewGuides::new(Arc::new(store_running(Some(run()))));
    let err = submit(&tasks(), &guides, &scope(), "task-1", request(50, 60))
        .await
        .unwrap_err();
    assert!(matches!(err, ReviewGuideError::BadRequest(_)), "{err:?}");
}

#[tokio::test]
async fn a_held_guide_is_stored_for_the_run_head() {
    let mut store = store_running(Some(run()));
    store
        .expect_complete_run()
        .withf(|run_id, guide| {
            run_id == "run-1"
                && guide.head_sha == "head"
                && guide.task_id == "task-1"
                && guide.guide["stops"][0]["excerpts"][0]["end"] == 4
        })
        .returning(|_, _| Ok(true));
    let guides = ReviewGuides::new(Arc::new(store));
    let response = submit(&tasks(), &guides, &scope(), "task-1", request(2, 4))
        .await
        .unwrap();
    assert_eq!((response.stops, response.excerpts), (1, 1));
}

#[tokio::test]
async fn a_run_replaced_before_submission_stores_nothing() {
    let mut store = store_running(Some(run()));
    store.expect_complete_run().returning(|_, _| Ok(false));
    let guides = ReviewGuides::new(Arc::new(store));
    let err = submit(&tasks(), &guides, &scope(), "task-1", request(2, 4))
        .await
        .unwrap_err();
    assert!(matches!(err, ReviewGuideError::NoRun(_)), "{err:?}");
}
