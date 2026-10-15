use moatless_types::pull_request_conversation::{
    PullRequestActivity, PullRequestActor, PullRequestCommitEntry, PullRequestRemark,
    PullRequestRemarkKind, ReviewThread, ReviewThreadComment,
};
use serde_json::json;

use super::*;

fn alice() -> PullRequestActor {
    PullRequestActor {
        login: "alice".into(),
        name: Some("Alice".into()),
        avatar_url: None,
        is_bot: false,
    }
}

#[test]
fn activity_takes_t3s_wire_shape() {
    let comment = ReviewThreadComment {
        id: "rc-1".into(),
        author: Some(alice()),
        body: "Why?".into(),
        created_at: "2026-10-01T09:00:00Z".into(),
        edited_at: None,
        url: None,
    };
    let activity = PullRequestActivity {
        author: Some(alice()),
        reviewers: vec![],
        remarks: vec![PullRequestRemark {
            id: "rc-1".into(),
            kind: PullRequestRemarkKind::ReviewComment,
            author: Some(alice()),
            body: "Why?".into(),
            created_at: "2026-10-01T09:00:00Z".into(),
            edited_at: None,
            url: None,
            path: Some("src/lib.rs".into()),
            review_state: None,
        }],
        remark_count: 3,
        remarks_truncated: true,
        review_threads: vec![ReviewThread {
            id: "th-1".into(),
            path: "src/lib.rs".into(),
            line: None,
            side: DiffSide::Left,
            is_resolved: true,
            is_outdated: false,
            comments: vec![comment],
            comment_count: 3,
            next_comments_cursor: Some("tc1".into()),
        }],
        review_threads_truncated: false,
        commits: vec![PullRequestCommitEntry {
            oid: "abc".into(),
            message_headline: "Add widgets".into(),
            committed_date: "2026-10-01T08:00:00Z".into(),
            additions: Some(3),
            deletions: None,
            authors: vec![alice()],
        }],
    };
    let value = activity_json(&activity);
    assert_eq!(value["comments"][0]["kind"], "review-comment");
    assert_eq!(value["comments"][0]["path"], "src/lib.rs");
    assert_eq!(value["commentCount"], 3);
    assert_eq!(value["commentsTruncated"], true);
    let thread = &value["reviewThreads"][0];
    assert_eq!(thread["side"], "left");
    assert_eq!(thread["line"], serde_json::Value::Null);
    assert_eq!(thread["nextCommentsCursor"], "tc1");
    assert_eq!(thread["comments"][0]["author"]["login"], "alice");
    assert_eq!(value["commits"][0]["additions"], 3);
    assert!(value["commits"][0].get("deletions").is_none());
    assert!(value["author"].get("isBot").is_none());
}

#[test]
fn a_file_read_names_its_change_and_both_paths() {
    let payload = json!({
        "changeType": "rename-changed",
        "oldPath": "src/old.rs",
        "newPath": "src/new.rs",
        "commit": "",
    });
    let file = changed_file(&payload).unwrap();
    assert_eq!(file.change, FileChangeType::RenameChanged);
    assert_eq!((file.old_path, file.new_path), ("src/old.rs", "src/new.rs"));
    assert_eq!(file.commit, None);
    assert!(
        changed_file(&json!({ "changeType": "copy", "oldPath": "a", "newPath": "a" })).is_none()
    );
}

#[test]
fn only_a_commit_id_names_a_commit() {
    assert!(is_commit_sha("0123abc"));
    assert!(is_commit_sha(&"f".repeat(40)));
    assert!(!is_commit_sha("main"));
    assert!(!is_commit_sha("abc"));
    assert!(!is_commit_sha("HEAD~1"));
}
