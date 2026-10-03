use std::sync::Arc;

use chrono::Utc;
use moatless_core::config::S3Config;

use super::*;
use crate::artifact::dao::MockArtifactDao;
use crate::task::dao::MockTaskDao;
use crate::task::test_support::make_task_row;

fn scope() -> ScopeContext {
    ScopeContext {
        user_id: "user-1".to_string(),
        is_admin: false,
    }
}

fn s3_config(bucket: Option<&str>) -> Arc<S3Config> {
    Arc::new(S3Config {
        bucket_name: bucket.map(str::to_string),
        key_prefix: "uploads".to_string(),
        transcript_prefix: "transcripts".to_string(),
        workspace_prefix: "workspaces".to_string(),
        artifact_prefix: "artifacts".to_string(),
        region: "us-east-1".to_string(),
    })
}

fn artifacts(store: MockArtifactDao) -> Artifacts {
    Artifacts::new(
        Arc::new(store),
        None,
        s3_config(None),
        "https://moatless.example/",
    )
}

/// A `TaskCommands` where the caller owns `task-1` and can read, but not
/// change, `task-shared`; every other Task is unreadable.
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

fn row(task_id: &str, latest_version: i32) -> Artifact {
    Artifact {
        artifact_id: "art-1".to_string(),
        task_id: task_id.to_string(),
        slug: "report".to_string(),
        title: "Report".to_string(),
        reserved_version: latest_version.max(1),
        latest_version,
        published_versions: (1..=latest_version).collect(),
        created_by: Some("user-1".to_string()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

fn store_with(row: Option<Artifact>) -> MockArtifactDao {
    let mut store = MockArtifactDao::new();
    store
        .expect_find_by_id()
        .returning(move |_| Ok(row.clone()));
    store
}

fn page() -> PublishRequest {
    PublishRequest {
        title: "Report".to_string(),
        file_name: Some("report.html".to_string()),
        html: b"<p>hi</p>".to_vec(),
        ..Default::default()
    }
}

#[test]
fn slug_is_derived_from_the_title_when_omitted() {
    assert_eq!(slug_from_title("Q3 Revenue — Draft!"), "q3-revenue-draft");
    assert_eq!(slug_from_title("  --  "), "artifact");
    assert_eq!(slug_from_title(&"x".repeat(100)).len(), MAX_SLUG_LEN);
}

#[test]
fn an_explicit_slug_must_already_be_well_formed() {
    assert!(validate_slug("release-notes-2").is_ok());
    for bad in ["-lead", "Upper", "under_score", &"a".repeat(64)] {
        assert!(validate_slug(bad).is_err(), "{bad:?} should be rejected");
    }
}

#[test]
fn the_page_must_be_one_non_empty_html_file_within_the_limit() {
    for name in ["report.html", "REPORT.HTM", "dir/page.html"] {
        assert!(check_page(Some(name), b"<p>").is_ok(), "{name:?}");
    }
    for name in ["style.css", "html", "report.html.exe", ""] {
        assert!(
            matches!(
                check_page(Some(name), b"<p>"),
                Err(ArtifactError::BadRequest(_))
            ),
            "{name:?}"
        );
    }
    assert!(matches!(
        check_page(None, b"<p>"),
        Err(ArtifactError::BadRequest(_))
    ));
    assert!(matches!(
        check_page(Some("a.html"), b""),
        Err(ArtifactError::BadRequest(_))
    ));
    assert!(matches!(
        check_page(Some("a.html"), &vec![b'x'; MAX_PAGE_BYTES + 1]),
        Err(ArtifactError::PayloadTooLarge(_))
    ));
}

#[tokio::test]
async fn publish_without_object_storage_is_refused_before_reserving() {
    // No `expect_reserve_version`: reserving would panic the mock.
    let err = publish(
        &tasks(),
        &artifacts(MockArtifactDao::new()),
        &scope(),
        "task-1",
        page(),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ArtifactError::StorageNotConfigured));
}

#[tokio::test]
async fn publish_on_a_task_the_caller_can_only_read_is_forbidden() {
    let err = publish(
        &tasks(),
        &artifacts(MockArtifactDao::new()),
        &scope(),
        "task-shared",
        page(),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ArtifactError::Forbidden(_)), "{err:?}");
}

#[tokio::test]
async fn latest_location_names_the_latest_version() {
    let location = latest_location(
        &tasks(),
        &artifacts(store_with(Some(row("task-1", 3)))),
        &scope(),
        "art-1",
    )
    .await
    .unwrap();
    assert_eq!(location, "/api/v1/artifacts/art-1/v3");
}

#[tokio::test]
async fn an_artifact_on_an_unreadable_task_is_not_found() {
    let err = latest_location(
        &tasks(),
        &artifacts(store_with(Some(row("task-other", 1)))),
        &scope(),
        "art-1",
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ArtifactError::NotFound(_)));
}

#[tokio::test]
async fn an_artifact_with_no_published_version_is_not_found() {
    let err = latest_location(
        &tasks(),
        &artifacts(store_with(Some(row("task-1", 0)))),
        &scope(),
        "art-1",
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ArtifactError::NotFound(_)));
}

#[tokio::test]
async fn a_version_past_the_latest_or_malformed_is_not_found() {
    for version in ["v3", "v0", "2", "vx"] {
        let err = read_page(
            &tasks(),
            &artifacts(store_with(Some(row("task-1", 2)))),
            &scope(),
            "art-1",
            version,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(err, ArtifactError::NotFound(_)),
            "{version}: {err:?}"
        );
    }
}

#[tokio::test]
async fn a_version_below_the_latest_that_never_committed_is_not_found() {
    let mut never_committed = row("task-1", 2);
    never_committed.published_versions = vec![2];
    let err = read_page(
        &tasks(),
        &artifacts(store_with(Some(never_committed))),
        &scope(),
        "art-1",
        "v1",
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ArtifactError::NotFound(_)), "{err:?}");
}

#[tokio::test]
async fn a_commit_error_after_the_commit_landed_answers_the_row() {
    let row = recover_commit(
        &artifacts(store_with(Some(row("task-1", 2)))),
        "task-1",
        "art-1",
        2,
        sqlx::Error::PoolTimedOut,
    )
    .await
    .unwrap();
    assert_eq!(row.latest_version, 2);
}

#[tokio::test]
async fn a_commit_error_with_the_version_unpublished_is_an_error() {
    let err = recover_commit(
        &artifacts(store_with(Some(row("task-1", 1)))),
        "task-1",
        "art-1",
        2,
        sqlx::Error::PoolTimedOut,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ArtifactError::Db(_)), "{err:?}");
}

#[tokio::test]
async fn a_commit_error_with_an_unreadable_row_keeps_the_commit_error() {
    let mut store = MockArtifactDao::new();
    store
        .expect_find_by_id()
        .returning(|_| Err(sqlx::Error::RowNotFound));
    let err = recover_commit(
        &artifacts(store),
        "task-1",
        "art-1",
        2,
        sqlx::Error::PoolTimedOut,
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, ArtifactError::Db(sqlx::Error::PoolTimedOut)),
        "{err:?}"
    );
}

#[tokio::test]
async fn a_commit_error_with_the_row_gone_is_task_not_found() {
    let err = recover_commit(
        &artifacts(store_with(None)),
        "task-1",
        "art-1",
        2,
        sqlx::Error::PoolTimedOut,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ArtifactError::TaskNotFound(_)), "{err:?}");
}

#[tokio::test]
async fn list_answers_absolute_urls_for_the_latest_version() {
    let mut store = MockArtifactDao::new();
    store
        .expect_list_for_task()
        .returning(|_| Ok(vec![row("task-1", 2)]));
    let items = list(&tasks(), &artifacts(store), &scope(), "task-1")
        .await
        .unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].version, 2);
    assert_eq!(
        items[0].url,
        "https://moatless.example/api/v1/artifacts/art-1/v2"
    );
    assert_eq!(
        items[0].latest_url,
        "https://moatless.example/api/v1/artifacts/art-1"
    );
}

#[tokio::test]
async fn list_on_an_unreadable_task_is_task_not_found() {
    let err = list(
        &tasks(),
        &artifacts(MockArtifactDao::new()),
        &scope(),
        "task-other",
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ArtifactError::TaskNotFound(_)));
}
