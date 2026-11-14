use moatless_types::pull_request_conversation::{
    DiffSide, MergeMethod, PullRequestAction, PullRequestWrite, ReviewPosition, ReviewVerdict,
};
use serde_json::json;

use super::{WriteInput, WriteMethod};

fn input(body: serde_json::Value) -> WriteInput {
    let mut value = json!({ "projectId": "ws-1", "repository": "acme/app", "number": 7 });
    value
        .as_object_mut()
        .unwrap()
        .extend(body.as_object().unwrap().clone());
    serde_json::from_value(value).expect("the input decodes")
}

#[test]
fn a_write_still_names_its_pull_request() {
    let input = input(json!({ "body": "hi" }));
    assert_eq!(
        input.reference.target(),
        Some(("ws-1".to_string(), "acme/app".to_string(), 7))
    );
}

#[test]
fn a_review_anchors_each_comment_on_its_side() {
    let write = input(json!({
        "verdict": "approve",
        "body": "",
        "comments": [
            { "path": "a.rs", "position": { "kind": "deleted", "oldLine": 4 }, "body": "gone" },
            {
                "path": "b.rs",
                "position": { "kind": "context", "oldLine": 3, "newLine": 5, "side": "right" },
                "startPosition": { "kind": "added", "newLine": 2 },
                "body": "range",
            },
        ],
    }))
    .into_write(WriteMethod::SubmitReview)
    .unwrap();
    let PullRequestWrite::SubmitReview {
        verdict, comments, ..
    } = write
    else {
        panic!("expected a review");
    };
    assert_eq!(verdict, ReviewVerdict::Approve);
    assert_eq!(
        comments[0].position,
        ReviewPosition {
            line: 4,
            side: DiffSide::Left
        }
    );
    assert_eq!(
        comments[1].position,
        ReviewPosition {
            line: 5,
            side: DiffSide::Right
        }
    );
    assert_eq!(
        comments[1].start,
        Some(ReviewPosition {
            line: 2,
            side: DiffSide::Right
        })
    );
}

#[test]
fn an_action_carries_its_method() {
    let merge = input(json!({ "action": "merge", "mergeMethod": "squash" }))
        .into_write(WriteMethod::RunAction)
        .unwrap();
    assert_eq!(
        merge,
        PullRequestWrite::Action(PullRequestAction::Merge(MergeMethod::Squash))
    );
    let update = input(json!({ "action": "update-branch", "updateMethod": "rebase" }))
        .into_write(WriteMethod::RunAction)
        .unwrap();
    assert_eq!(
        update,
        PullRequestWrite::Action(PullRequestAction::UpdateBranch { rebase: true })
    );
}

#[test]
fn an_action_this_backend_does_not_serve_is_refused() {
    for action in ["revert", "approve-workflows"] {
        assert!(
            input(json!({ "action": action }))
                .into_write(WriteMethod::RunAction)
                .is_err()
        );
    }
    assert!(
        input(json!({ "action": "merge", "mergeMethod": "fast-forward" }))
            .into_write(WriteMethod::RunAction)
            .is_err()
    );
}

#[test]
fn a_team_reviewer_is_requested_by_its_slug() {
    let write = input(json!({
        "reviewers": [{ "id": "alice", "kind": "user" }, { "id": "acme/core", "kind": "team" }],
        "requested": true,
    }))
    .into_write(WriteMethod::RequestReviewers)
    .unwrap();
    assert_eq!(
        write,
        PullRequestWrite::RequestReviewers {
            users: vec!["alice".into()],
            teams: vec!["core".into()],
            requested: true,
        }
    );
}

#[test]
fn an_empty_comment_is_refused() {
    assert!(
        input(json!({ "body": "" }))
            .into_write(WriteMethod::Comment)
            .is_err()
    );
}
