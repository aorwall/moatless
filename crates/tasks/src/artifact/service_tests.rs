use std::sync::Arc;

use chrono::Utc;
use moatless_core::config::S3Config;

use super::*;
use crate::artifact::models::DataAccess;
use crate::artifact::test_support::{TestStore, artifacts_over};
use crate::task::dao::MockTaskDao;
use crate::task::test_support::make_task_row;
use moatless_types::task::Visibility;

fn scope() -> ScopeContext {
    ScopeContext::user("user-1")
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

fn artifacts(store: TestStore) -> Artifacts {
    artifacts_over(
        store,
        s3_config(None),
        "https://moatless.example/",
        Some("page-key"),
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
        inline_versions: Vec::new(),
        accepts_data: false,
        notify_on_data: false,
        record_rules: None,
        collections: Vec::new(),
        visibility: Visibility::Private,
        forked_from_artifact_id: None,
        forked_from_version: None,
        created_by: Some("user-1".to_string()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

fn store_with(row: Option<Artifact>) -> TestStore {
    let mut store = TestStore::default();
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
        &artifacts(TestStore::default()),
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
        &artifacts(TestStore::default()),
        &scope(),
        "task-shared",
        page(),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ArtifactError::Forbidden(_)), "{err:?}");
}

#[tokio::test]
async fn no_version_names_the_latest_version() {
    let (_, version) = find_version(
        &tasks(),
        &artifacts(store_with(Some(row("task-1", 3)))),
        &scope(),
        "art-1",
        None,
    )
    .await
    .unwrap();
    assert_eq!(version, 3);
}

#[tokio::test]
async fn an_artifact_on_an_unreadable_task_is_not_found() {
    let err = find_version(
        &tasks(),
        &artifacts(store_with(Some(row("task-other", 1)))),
        &scope(),
        "art-1",
        None,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ArtifactError::NotFound(_)));
}

#[tokio::test]
async fn a_public_artifact_on_an_unreadable_task_is_readable() {
    let mut public = row("task-other", 2);
    public.visibility = Visibility::Public;
    let (_, version) = find_version(
        &tasks(),
        &artifacts(store_with(Some(public))),
        &scope(),
        "art-1",
        None,
    )
    .await
    .unwrap();
    assert_eq!(version, 2);
}

#[tokio::test]
async fn an_artifact_with_no_published_version_is_not_found() {
    let err = find_version(
        &tasks(),
        &artifacts(store_with(Some(row("task-1", 0)))),
        &scope(),
        "art-1",
        None,
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
            Some(version),
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
        Some("v1"),
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
    let mut store = TestStore::default();
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
    let mut store = TestStore::default();
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
        &artifacts(TestStore::default()),
        &scope(),
        "task-other",
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ArtifactError::TaskNotFound(_)));
}

#[tokio::test]
async fn notify_without_data_is_refused() {
    let mut req = page();
    req.data.notify_on_data = true;
    let err = publish(
        &tasks(),
        &artifacts(TestStore::default()),
        &scope(),
        "task-1",
        req,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ArtifactError::BadRequest(_)), "{err:?}");
}

#[tokio::test]
async fn rules_need_data_and_replace_notify() {
    let rules = crate::artifact::rules::RecordRules::parse(
        r#"{"rulesVersion": 1, "prefixes": {"votes/": {"create": "anyone"}}}"#,
    )
    .unwrap();
    let mut without_data = page();
    without_data.data.rules = Some(rules.clone());
    let mut with_notify = page();
    with_notify.data = DataAccess {
        accepts_data: true,
        notify_on_data: true,
        rules: Some(rules),
        collections: Vec::new(),
    };
    for (req, wanted) in [
        (without_data, "rules need data"),
        (with_notify, "rules replace notify"),
    ] {
        let err = publish(
            &tasks(),
            &artifacts(TestStore::default()),
            &scope(),
            "task-1",
            req,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(&err, ArtifactError::BadRequest(m) if m.contains(wanted)),
            "{err:?}"
        );
    }
}

#[tokio::test]
async fn the_owner_sets_an_artifacts_visibility() {
    let mut store = store_with(Some(row("task-1", 2)));
    store
        .expect_set_visibility()
        .withf(|id, visibility| id == "art-1" && *visibility == Visibility::Public)
        .times(1)
        .returning(|_, visibility| {
            let mut row = row("task-1", 2);
            row.visibility = visibility;
            Ok(Some(row))
        });
    let response = set_visibility(
        &tasks(),
        &artifacts(store),
        &scope(),
        "art-1",
        Visibility::Public,
    )
    .await
    .unwrap();
    assert_eq!(response.visibility, Visibility::Public);
    assert_eq!(response.version, 2);
}

#[tokio::test]
async fn a_reader_who_cannot_change_the_task_cannot_set_visibility() {
    let mut store = store_with(Some(row("task-shared", 1)));
    store.expect_set_visibility().never();
    let err = set_visibility(
        &tasks(),
        &artifacts(store),
        &scope(),
        "art-1",
        Visibility::Public,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ArtifactError::Forbidden(_)), "{err:?}");
}

#[tokio::test]
async fn a_public_artifact_on_an_unreadable_task_is_not_found_to_set() {
    let mut public = row("task-other", 1);
    public.visibility = Visibility::Public;
    let mut store = store_with(Some(public));
    store.expect_set_visibility().never();
    let err = set_visibility(
        &tasks(),
        &artifacts(store),
        &scope(),
        "art-1",
        Visibility::Private,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ArtifactError::NotFound(_)), "{err:?}");
}

fn global_collection(id: &str) -> crate::collection::models::CollectionRow {
    crate::collection::models::CollectionRow {
        collection_id: id.to_string(),
        scope_kind: "global".to_string(),
        scope_id: String::new(),
        name: "runs".to_string(),
        created_at: Utc::now(),
        record_count: 0,
        last_updated_at: None,
    }
}

#[tokio::test]
async fn a_publisher_who_cannot_write_a_scope_attaches_only_an_existing_collection() {
    let task = make_task_row("task-1", "user-1");
    let specs = ["global:runs".to_string()];

    // No `expect_ensure_collection`: calling it fails the test.
    let mut store = TestStore::default();
    store
        .records
        .expect_find_collection()
        .returning(|_, _, _| Ok(Some(global_collection("col-1"))));
    let attached = attach(&artifacts(store), &scope(), &task, &specs)
        .await
        .unwrap();
    assert_eq!(attached[0].collection_id, "col-1");

    let mut store = TestStore::default();
    store
        .records
        .expect_find_collection()
        .returning(|_, _, _| Ok(None));
    let err = attach(&artifacts(store), &scope(), &task, &specs)
        .await
        .unwrap_err();
    assert!(matches!(err, ArtifactError::BadRequest(_)), "{err:?}");
}

#[tokio::test]
async fn a_publisher_who_may_write_a_scope_creates_the_collection() {
    let mut store = TestStore::default();
    store
        .records
        .expect_ensure_collection()
        .times(1)
        .returning(|_, _, _, _, _, _| Ok(global_collection("col-new")));
    let admin = ScopeContext::new("user-1".to_string(), true);
    let attached = attach(
        &artifacts(store),
        &admin,
        &make_task_row("task-1", "user-1"),
        &["global:runs".to_string()],
    )
    .await
    .unwrap();
    assert_eq!(attached[0].collection_id, "col-new");
}

fn store_listing_versions(row: Artifact, versions: Vec<ArtifactVersion>) -> TestStore {
    let mut store = store_with(Some(row));
    store
        .expect_list_versions()
        .returning(move |_| Ok(versions.clone()));
    store
}

#[tokio::test]
async fn the_owner_views_a_version_with_its_task_and_can_manage_it() {
    let published = ArtifactVersion {
        version: 2,
        published_at: Utc::now(),
        turn_number: Some(4),
    };
    let view = page_view(
        &tasks(),
        &artifacts(store_listing_versions(
            row("task-1", 3),
            vec![published.clone()],
        )),
        &scope(),
        "art-1",
        Some("v2"),
    )
    .await
    .unwrap();
    assert_eq!(view.version, 2);
    assert_eq!(view.versions, vec![published]);
    assert!(view.task_readable);
    assert!(view.can_manage);
    assert!(!view.source_readable);
}

#[tokio::test]
async fn a_reader_who_cannot_change_the_task_views_it_without_managing() {
    let view = page_view(
        &tasks(),
        &artifacts(store_listing_versions(row("task-shared", 1), Vec::new())),
        &scope(),
        "art-1",
        None,
    )
    .await
    .unwrap();
    assert!(view.task_readable);
    assert!(!view.can_manage);
}

#[tokio::test]
async fn a_public_page_on_an_unreadable_task_is_viewed_without_the_task() {
    let mut public = row("task-other", 1);
    public.visibility = Visibility::Public;
    let view = page_view(
        &tasks(),
        &artifacts(store_listing_versions(public, Vec::new())),
        &scope(),
        "art-1",
        None,
    )
    .await
    .unwrap();
    assert!(!view.task_readable);
    assert!(!view.can_manage);
}

#[tokio::test]
async fn a_fork_names_its_source_as_readable_only_when_it_is() {
    for (source_task, readable) in [("task-1", true), ("task-other", false)] {
        let mut fork = row("task-1", 1);
        fork.artifact_id = "art-fork".to_string();
        fork.forked_from_artifact_id = Some("art-1".to_string());
        fork.forked_from_version = Some(2);
        let source = row(source_task, 2);
        let mut store = TestStore::default();
        store.expect_find_by_id().returning(move |id| {
            Ok(Some(if id == "art-fork" {
                fork.clone()
            } else {
                source.clone()
            }))
        });
        store.expect_list_versions().returning(|_| Ok(Vec::new()));
        let view = page_view(&tasks(), &artifacts(store), &scope(), "art-fork", None)
            .await
            .unwrap();
        assert_eq!(view.source_readable, readable, "{source_task}");
    }
}
