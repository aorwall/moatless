use chrono::Utc;
use moatless_core::config::S3Config;
use moatless_types::task::Visibility;

use super::*;
use crate::artifact::dao::MockArtifactDao;
use crate::artifact::rules::RecordRules;
use crate::task::dao::MockTaskDao;
use crate::task::test_support::make_task_row;
use moatless_api_schemas::artifact::RecordWriter;

fn record(key: &str, submitted_by: &str) -> ArtifactRecord {
    ArtifactRecord {
        artifact_id: "art-1".to_string(),
        key: key.to_string(),
        data: json!({}),
        submitted_by: submitted_by.to_string(),
        writer: RecordWriter::Page,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

fn subscription(prefix: Option<&str>, cursor: i64, shown: &[&str]) -> Subscription {
    Subscription {
        id: json!("1"),
        prefix: prefix.map(str::to_string),
        cursor,
        shown: shown.iter().map(|k| (*k).to_string()).collect(),
        stale: false,
    }
}

fn changes(entries: &[(i64, &str)]) -> Vec<(i64, String)> {
    entries
        .iter()
        .map(|(s, k)| (*s, (*k).to_string()))
        .collect()
}

fn keys(records: &[ArtifactRecord]) -> Vec<&str> {
    records.iter().map(|r| r.key.as_str()).collect()
}

#[test]
fn a_page_hears_each_changed_key_once_and_the_removal_of_one_it_held() {
    let mut sub = subscription(Some("req/"), 4, &["req/gone"]);
    let saved = record("req/1", "user-1");
    let current = HashMap::from([("req/1", &saved)]);
    let log = changes(&[
        (3, "req/old"),
        (5, "req/1"),
        (6, "other"),
        (7, "req/gone"),
        (8, "req/never-seen"),
        (9, "req/1"),
    ]);
    let Advance::Changes { records, removed } = sub.advance(9, &log, &current, |_| true) else {
        panic!("expected changes");
    };
    assert_eq!(keys(&records), vec!["req/1"]);
    assert_eq!(removed, vec!["req/gone".to_string()]);
    assert_eq!(sub.cursor, 9);
    assert!(sub.shown.contains("req/1") && !sub.shown.contains("req/gone"));
    assert!(matches!(
        sub.advance(9, &log, &current, |_| true),
        Advance::Nothing
    ));
}

#[test]
fn a_page_whose_cursor_the_log_no_longer_reaches_reads_everything_again() {
    let mut sub = subscription(None, 2, &[]);
    let log = changes(&[(5, "a"), (6, "b")]);
    assert!(matches!(
        sub.advance(6, &log, &HashMap::new(), |_| true),
        Advance::Resync
    ));
    let mut pruned_to_nothing = subscription(None, 2, &[]);
    assert!(matches!(
        pruned_to_nothing.advance(6, &[], &HashMap::new(), |_| true),
        Advance::Resync
    ));
}

#[test]
fn a_record_the_page_may_no_longer_read_is_removed_from_it() {
    let mut sub = subscription(None, 0, &["draft"]);
    let theirs = record("draft", "user-2");
    let current = HashMap::from([("draft", &theirs)]);
    let Advance::Changes { records, removed } =
        sub.advance(1, &changes(&[(1, "draft")]), &current, |r| {
            r.submitted_by == "user-1"
        })
    else {
        panic!("expected changes");
    };
    assert!(records.is_empty());
    assert_eq!(removed, vec!["draft".to_string()]);
}

fn connection(store: MockArtifactDao, rules: Option<&str>) -> Connection {
    let mut tasks = MockTaskDao::new();
    tasks
        .expect_find_by_id()
        .returning(|task_id, _| Ok(Some(make_task_row(task_id, "user-1"))));
    Connection {
        tasks: Arc::new(TaskCommands::new(Arc::new(tasks))),
        artifacts: Arc::new(Artifacts::new(
            Arc::new(store),
            None,
            Arc::new(S3Config {
                bucket_name: None,
                key_prefix: "uploads".to_string(),
                transcript_prefix: "transcripts".to_string(),
                workspace_prefix: "workspaces".to_string(),
                artifact_prefix: "artifacts".to_string(),
                region: "us-east-1".to_string(),
            }),
            "https://moatless.example",
            Some("page-key".to_string()),
        )),
        token: String::new(),
        viewer: "user-1".to_string(),
        expires_at: 0,
        row: Artifact {
            artifact_id: "art-1".to_string(),
            task_id: "task-1".to_string(),
            slug: "survey".to_string(),
            title: "Survey".to_string(),
            reserved_version: 1,
            latest_version: 1,
            published_versions: vec![1],
            inline_versions: Vec::new(),
            accepts_data: true,
            notify_on_data: false,
            record_rules: rules.map(|r| RecordRules::parse(r).unwrap()),
            visibility: Visibility::Private,
            created_by: Some("user-1".to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        },
        subscriptions: Vec::new(),
    }
}

fn frames(inbound: Inbound) -> Vec<Value> {
    match inbound {
        Inbound::Frames(frames) => frames,
        Inbound::End(_) | Inbound::Pong(_) => panic!("expected frames to carry on with"),
    }
}

#[tokio::test]
async fn a_subscription_starts_with_what_the_rules_show_then_streams_changes() {
    let mut store = MockArtifactDao::new();
    store
        .expect_record_heads()
        .returning(|_| Ok(vec![("art-1".to_string(), 3)]));
    store.expect_list_records().returning(|_, _, _| {
        Ok(vec![
            record("drafts/mine", "user-1"),
            record("drafts/theirs", "user-2"),
        ])
    });
    store
        .expect_record_changes()
        .withf(|_, after| *after == 3)
        .returning(|_, _| Ok((5, changes(&[(4, "drafts/new"), (5, "drafts/mine")]))));
    store.expect_find_records().returning(|_, _| {
        Ok(vec![
            record("drafts/new", "user-1"),
            record("drafts/mine", "user-1"),
        ])
    });
    let mut connection = connection(
        store,
        Some(r#"{"rulesVersion": 1, "prefixes": {"drafts/": {"read": "owner"}}}"#),
    );

    let subscribe =
        json!({"_tag": "Request", "id": "7", "tag": SUBSCRIBE, "payload": {"prefix": "drafts/"}});
    let sent = frames(
        connection
            .on_message(subscribe.to_string().as_bytes())
            .await,
    );
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["requestId"], "7");
    let snapshot = &sent[0]["values"][0];
    assert_eq!(
        (snapshot["_tag"].as_str(), snapshot["done"].as_bool()),
        (Some("Snapshot"), Some(true))
    );
    assert_eq!(snapshot["items"].as_array().unwrap().len(), 1);
    assert_eq!(snapshot["items"][0]["key"], "drafts/mine");

    let pushed = connection.pull().await;
    let change = &pushed[0]["values"][0];
    assert_eq!(change["_tag"], "Changes");
    let pushed_keys: Vec<_> = change["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["key"].as_str().unwrap())
        .collect();
    assert_eq!(pushed_keys, vec!["drafts/new", "drafts/mine"]);

    let interrupt = json!({"_tag": "Interrupt", "requestId": "7"});
    let sent = frames(
        connection
            .on_message(interrupt.to_string().as_bytes())
            .await,
    );
    assert_eq!(sent[0]["exit"]["cause"][0]["_tag"], "Interrupt");
    assert!(connection.pull().await.is_empty());
}

#[tokio::test]
async fn the_socket_answers_pings_and_refuses_what_it_does_not_serve() {
    let mut connection = connection(MockArtifactDao::new(), None);
    let batch = json!([
        {"_tag": "Ping"},
        {"_tag": "Request", "id": "1", "tag": "artifact.records.write", "payload": {}},
        {"_tag": "Ack", "requestId": "1"},
        {"_tag": "Nonsense", "x": 1},
    ]);
    let sent = frames(connection.on_message(batch.to_string().as_bytes()).await);
    assert_eq!(sent[0]["_tag"], "Pong");
    assert_eq!(sent[1]["exit"]["cause"][0]["_tag"], "Die");
    assert_eq!(sent.len(), 2);

    let sent = frames(connection.on_message(b"not json").await);
    assert_eq!(sent[0]["_tag"], "Defect");
    assert!(matches!(
        connection.on_message(br#"{"_tag": "Eof"}"#).await,
        Inbound::End(_)
    ));
}

#[test]
fn a_subscription_ends_with_the_code_the_page_script_rejects_with() {
    let frame = failed(&json!("3"), &ArtifactError::Unauthorized);
    assert_eq!(frame["requestId"], "3");
    assert_eq!(
        frame["exit"]["cause"][0]["error"]["code"],
        "unauthenticated"
    );
}

#[tokio::test]
async fn a_resync_that_fails_keeps_the_subscription_where_it_was_and_sends_nothing() {
    let mut store = MockArtifactDao::new();
    store
        .expect_record_changes()
        .returning(|_, _| Ok((9, changes(&[(8, "a"), (9, "a")]))));
    store.expect_find_records().returning(|_, _| Ok(Vec::new()));
    store
        .expect_record_heads()
        .returning(|_| Err(sqlx::Error::PoolTimedOut));
    let mut connection = connection(store, None);
    connection.subscriptions.push(subscription(None, 2, &[]));

    assert!(connection.pull().await.is_empty());
    assert_eq!(connection.subscriptions.len(), 1);
    assert_eq!(connection.subscriptions[0].cursor, 2);
    // Even with no change past its cursor, the next pull resyncs it.
    assert!(matches!(
        connection.subscriptions[0].advance(2, &[], &HashMap::new(), |_| true),
        Advance::Resync
    ));
}

#[test]
fn only_a_lost_reader_ends_the_socket() {
    assert!(ends_access(&ArtifactError::Unauthorized));
    assert!(ends_access(&ArtifactError::Forbidden("task-1".to_string())));
    assert!(!ends_access(&ArtifactError::Db(sqlx::Error::PoolTimedOut)));
}

#[tokio::test]
async fn a_request_id_already_subscribed_is_refused() {
    let mut connection = connection(MockArtifactDao::new(), None);
    connection.subscriptions.push(subscription(None, 0, &[]));
    let again = json!({"_tag": "Request", "id": "1", "tag": SUBSCRIBE});
    let sent = frames(connection.on_message(again.to_string().as_bytes()).await);
    assert_eq!(sent[0]["exit"]["cause"][0]["_tag"], "Die");
    assert_eq!(connection.subscriptions.len(), 1);
}
