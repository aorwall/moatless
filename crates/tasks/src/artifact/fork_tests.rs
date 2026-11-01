use chrono::Utc;

use super::*;
use moatless_types::task::Visibility;

fn source() -> Artifact {
    Artifact {
        artifact_id: "art-1".to_string(),
        task_id: "task-1".to_string(),
        slug: "report".to_string(),
        title: "Bob's report".to_string(),
        reserved_version: 4,
        latest_version: 4,
        published_versions: vec![1, 2, 3, 4],
        inline_versions: Vec::new(),
        accepts_data: true,
        notify_on_data: true,
        record_rules: None,
        collections: Vec::new(),
        visibility: Visibility::Public,
        forked_from_artifact_id: None,
        forked_from_version: None,
        created_by: Some("user-1".to_string()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

fn copy(accepts_data: bool) -> ArtifactResponse {
    ArtifactResponse {
        artifact_id: "art-2".to_string(),
        task_id: "task-2".to_string(),
        slug: "report".to_string(),
        title: "Bob's report".to_string(),
        version: 1,
        url: "https://moatless.example/api/v1/artifacts/art-2/v1".to_string(),
        latest_url: "https://moatless.example/api/v1/artifacts/art-2".to_string(),
        created_by: Some("user-2".to_string()),
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
        accepts_data,
        notify_on_data: false,
        visibility: Visibility::Private,
        record_rules: None,
        collections: Vec::new(),
    }
}

#[test]
fn the_brief_names_the_copy_to_fetch_and_republish() {
    let text = brief(&source(), 3, &copy(true), Some("Make it blue"), false);
    assert!(text.contains("\"Bob's report\" (v3) from another Task"));
    assert!(text.contains("https://moatless.example/api/v1/artifacts/art-2/v1"));
    assert!(text.contains("`moat artifact get art-2 -o report.html`"));
    assert!(text.contains(
        r"`moat artifact publish report.html --title 'Bob'\''s report' --slug report --data`"
    ));
    assert!(text.ends_with("Make it blue"));
}

#[test]
fn a_brief_in_a_fork_of_the_task_names_the_agents_own_artifact() {
    let text = brief(&source(), 3, &copy(false), None, true);
    assert!(text.starts_with("Your Artifact `report` (v3) was forked into this Task"));
    assert!(!text.contains("--data"));
    assert!(text.ends_with("Fetch the page and ask what to change."));
}

#[test]
fn a_title_is_one_shell_word() {
    assert_eq!(shell_quote("a 'b' $c"), r"'a '\''b'\'' $c'");
}
