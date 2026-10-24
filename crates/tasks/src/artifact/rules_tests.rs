use chrono::Utc;

use super::*;

const ADMIN_PAGE: &str = r#"{
  "rulesVersion": 1,
  "prefixes": {
    "requests/":  { "read": "anyone", "create": "anyone", "notify": ["create"] },
    "plans/":     { "read": "anyone" },
    "drafts/":    { "read": "owner", "create": "anyone", "update": "owner", "delete": "owner" },
    "drafts/shared/": { "read": "anyone", "create": "anyone", "update": "anyone" }
  }
}"#;

fn rules() -> RecordRules {
    RecordRules::parse(ADMIN_PAGE).unwrap()
}

fn record(key: &str, submitted_by: &str, writer: RecordWriter) -> ArtifactRecord {
    ArtifactRecord {
        artifact_id: "art-1".to_string(),
        key: key.to_string(),
        data: serde_json::json!({}),
        submitted_by: submitted_by.to_string(),
        writer,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

fn denied_by(prefix: &str) -> Result<(), Denied> {
    Err(Denied {
        prefix: Some(prefix.to_string()),
    })
}

#[test]
fn a_file_with_a_mistake_is_refused_with_what_is_wrong() {
    for (text, wanted) in [
        (r#"{"rulesVersion": 2, "prefixes": {}}"#, "rulesVersion 2"),
        (
            r#"{"rulesVersion": 1, "prefixes": {"a/": {"write": "anyone"}}}"#,
            "unknown field `write`",
        ),
        (
            r#"{"rulesVersion": 1, "prefixes": {"a/": {"read": "admins"}}}"#,
            "unknown variant `admins`",
        ),
        (
            r#"{"rulesVersion": 1, "prefixes": {"a/": {"create": "owner"}}}"#,
            "create cannot be \"owner\"",
        ),
        (
            r#"{"rulesVersion": 1, "prefixes": {"a/": {"create": "anyone", "notify": ["delete"]}}}"#,
            "notify names \"delete\"",
        ),
        (
            r#"{"rulesVersion": 1, "prefixes": {"a b/": {"read": "anyone"}}}"#,
            "prefix \"a b/\"",
        ),
        (
            r#"{"rulesVersion": 1, "prefixes": {"a//": {"read": "anyone"}}}"#,
            "prefix \"a//\"",
        ),
    ] {
        let err = RecordRules::parse(text).unwrap_err();
        assert!(err.contains(wanted), "{text}: {err}");
    }
}

#[test]
fn an_operation_a_rule_leaves_out_is_refused() {
    let rules = rules();
    let stored = record("requests/1", "user-1", RecordWriter::Page);
    assert_eq!(
        rules.check_write(WriteOp::Create, "requests/1", None, "user-1"),
        Ok(())
    );
    for op in [WriteOp::Update, WriteOp::Delete] {
        assert_eq!(
            rules.check_write(op, "requests/1", Some(&stored), "user-1"),
            denied_by("requests/"),
            "{op:?}"
        );
    }
    assert_eq!(
        rules.check_write(WriteOp::Create, "plans/1", None, "user-1"),
        denied_by("plans/")
    );
}

#[test]
fn a_key_no_prefix_matches_is_refused_and_an_empty_prefix_matches_every_key() {
    assert_eq!(
        rules().check_write(WriteOp::Create, "other", None, "user-1"),
        Err(Denied { prefix: None })
    );
    let open = RecordRules::parse(
        r#"{"rulesVersion": 1, "prefixes": {"": {"create": "anyone"}, "locked/": {}}}"#,
    )
    .unwrap();
    assert_eq!(
        open.check_write(WriteOp::Create, "other", None, "user-1"),
        Ok(())
    );
    assert_eq!(
        open.check_write(WriteOp::Create, "locked/1", None, "user-1"),
        denied_by("locked/")
    );
}

#[test]
fn the_longest_matching_prefix_decides() {
    let rules = rules();
    let theirs = record("drafts/shared/1", "user-2", RecordWriter::Page);
    assert_eq!(
        rules.check_write(WriteOp::Update, "drafts/shared/1", Some(&theirs), "user-1"),
        Ok(())
    );
    let theirs = record("drafts/1", "user-2", RecordWriter::Page);
    assert_eq!(
        rules.check_write(WriteOp::Update, "drafts/1", Some(&theirs), "user-1"),
        denied_by("drafts/")
    );
}

#[test]
fn owner_means_the_reader_who_wrote_the_stored_record() {
    let rules = rules();
    let mine = record("drafts/1", "user-1", RecordWriter::Page);
    assert_eq!(
        rules.check_write(WriteOp::Delete, "drafts/1", Some(&mine), "user-1"),
        Ok(())
    );
    assert_eq!(
        rules.check_write(WriteOp::Delete, "drafts/1", Some(&mine), "user-2"),
        denied_by("drafts/")
    );
}

#[test]
fn an_owner_read_rule_shows_the_reader_their_own_records_and_the_agents() {
    let rules = rules();
    assert!(rules.can_read(&record("drafts/1", "user-1", RecordWriter::Page), "user-1"));
    assert!(!rules.can_read(&record("drafts/1", "user-2", RecordWriter::Page), "user-1"));
    assert!(rules.can_read(&record("drafts/1", "user-2", RecordWriter::Agent), "user-1"));
    assert!(rules.can_read(&record("plans/1", "user-2", RecordWriter::Agent), "user-1"));
    assert!(!rules.can_read(&record("other", "user-1", RecordWriter::Page), "user-1"));
}

#[test]
fn only_the_writes_a_rule_lists_notify() {
    let rules = rules();
    assert!(rules.notifies(WriteOp::Create, "requests/1"));
    assert!(!rules.notifies(WriteOp::Create, "drafts/1"));
    assert!(!rules.notifies(WriteOp::Update, "drafts/1"));
    assert!(!rules.notifies(WriteOp::Create, "other"));
}

#[test]
fn rules_read_back_as_they_were_published() {
    let rules = rules();
    let value = serde_json::to_value(&rules).unwrap();
    assert_eq!(
        value["prefixes"]["requests/"],
        serde_json::json!({"read": "anyone", "create": "anyone", "notify": ["create"]})
    );
    assert_eq!(serde_json::from_value::<RecordRules>(value).unwrap(), rules);
}
