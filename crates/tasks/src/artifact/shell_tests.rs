use axum::http::HeaderValue;
use chrono::{TimeZone, Utc};

use super::*;
use crate::artifact::models::{Artifact, ArtifactVersion};

fn headers(dest: Option<&str>) -> HeaderMap {
    let mut headers = HeaderMap::new();
    if let Some(dest) = dest {
        headers.insert("sec-fetch-dest", HeaderValue::from_str(dest).unwrap());
    }
    headers
}

fn view(title: &str) -> PageView {
    PageView {
        row: Artifact {
            artifact_id: "art-1".to_string(),
            task_id: "task-1".to_string(),
            slug: "report".to_string(),
            title: title.to_string(),
            reserved_version: 3,
            latest_version: 3,
            published_versions: vec![1, 3, 2],
            inline_versions: Vec::new(),
            accepts_data: false,
            notify_on_data: false,
            record_rules: None,
            collections: Vec::new(),
            visibility: Visibility::Public,
            forked_from_artifact_id: Some("art-0".to_string()),
            forked_from_version: Some(4),
            created_by: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        },
        version: 2,
        versions: vec![ArtifactVersion {
            version: 2,
            published_at: Utc.with_ymd_and_hms(2026, 10, 1, 12, 0, 0).unwrap(),
            turn_number: Some(1),
        }],
        task_readable: false,
        can_manage: false,
        source_readable: false,
    }
}

/// The JSON the viewer's script reads.
fn data(html: &str) -> serde_json::Value {
    let start = html.find(r#"id="shell-data">"#).unwrap() + r#"id="shell-data">"#.len();
    let end = start + html[start..].find("</script>").unwrap();
    serde_json::from_str(&html[start..end]).unwrap()
}

#[test]
fn only_a_browser_opening_the_url_gets_the_viewer() {
    assert!(wanted(&headers(Some("document")), false, false));
    for dest in [None, Some("iframe"), Some("empty")] {
        assert!(!wanted(&headers(dest), false, false), "{dest:?}");
    }
    assert!(!wanted(&headers(Some("document")), true, false));
    assert!(!wanted(&headers(Some("document")), false, true));
}

#[test]
fn the_viewer_lists_every_published_version_in_order_with_its_date() {
    let html = render(&view("Report"), false, "https://ui.example", "n0nce");
    let data = data(&html);
    let versions = data["versions"].as_array().unwrap();
    let numbers: Vec<i64> = versions
        .iter()
        .map(|v| v["version"].as_i64().unwrap())
        .collect();
    assert_eq!(numbers, [1, 2, 3]);
    assert!(versions[0]["publishedAt"].is_null());
    assert_eq!(versions[1]["publishedAt"], "2026-10-01T12:00:00+00:00");
    assert_eq!(data["version"], 2);
    assert_eq!(data["latestVersion"], 3);
}

#[test]
fn an_unreadable_task_and_source_are_not_named() {
    let data = data(&render(&view("Report"), false, "https://ui.example", "n"));
    assert!(data["task"].is_null());
    assert_eq!(data["forkedFrom"]["version"], 4);
    assert!(data["forkedFrom"]["url"].is_null());
    assert!(!data.to_string().contains("task-1"));
}

#[test]
fn a_readable_task_and_source_are_linked() {
    let mut view = view("Report");
    view.task_readable = true;
    view.source_readable = true;
    let data = data(&render(&view, false, "https://ui.example", "n"));
    assert_eq!(data["task"]["url"], "https://ui.example/tasks/task-1");
    assert_eq!(data["forkedFrom"]["url"], "/api/v1/artifacts/art-0/v4");
}

#[test]
fn the_agents_title_cannot_break_out_of_the_title_or_the_data() {
    let title = "</title><script>alert(1)</script>";
    let html = render(&view(title), false, "https://ui.example", "n0nce");
    assert!(!html.contains("<script>alert"));
    assert_eq!(data(&html)["title"], title);
    assert_eq!(html.matches("nonce=\"n0nce\"").count(), 2);
}

#[test]
fn only_the_nonce_tagged_script_runs_in_the_viewer() {
    let csp = csp("abc");
    assert!(csp.contains("script-src 'nonce-abc'"));
    assert!(csp.contains("frame-ancestors 'none'"));
    assert!(csp.contains("frame-src 'self'"));
}

#[test]
fn a_title_naming_a_placeholder_is_shown_as_written() {
    let title = "__DATA__ and __NONCE__";
    let html = render(&view(title), false, "https://ui.example", "n0nce");
    assert!(html.contains("<title>__DATA__ and __NONCE__ · Moatless</title>"));
    assert_eq!(data(&html)["title"], title);
}

#[test]
fn the_viewer_knows_whether_a_fork_can_cut_at_the_publishing_turn() {
    let mut view = view("Report");
    assert_eq!(data(&render(&view, false, "u", "n"))["turnKnown"], true);
    view.version = 1;
    assert_eq!(data(&render(&view, false, "u", "n"))["turnKnown"], false);
}
