//! Reading a comment's own URL back to decide which pull request it is on.

use super::*;

#[test]
fn a_comment_url_names_the_pull_request_or_issue_it_is_on() {
    assert_eq!(
        trailing_number("https://api.github.com/repos/o/r/pulls/42"),
        Some(42)
    );
    assert_eq!(
        trailing_number("https://api.github.com/repos/o/r/issues/1318"),
        Some(1318)
    );
    assert_eq!(trailing_number(""), None);
    assert_eq!(
        trailing_number("https://api.github.com/repos/o/r/pulls/"),
        None
    );
}

#[test]
fn a_review_comment_response_carries_its_pull_request() {
    let comment: ReviewCommentRef = serde_json::from_str(
        r#"{"id":1,"pull_request_url":"https://api.github.com/repos/o/r/pulls/42"}"#,
    )
    .expect("a review comment");
    assert_eq!(trailing_number(&comment.pull_request_url), Some(42));
}

/// A conversation comment on a pull request is an issue comment, so its own URL
/// says `issues` where the pull request number is the same number.
#[test]
fn a_conversation_comment_response_carries_its_issue() {
    let comment: IssueCommentRef = serde_json::from_str(
        r#"{"id":1,"issue_url":"https://api.github.com/repos/o/r/issues/42"}"#,
    )
    .expect("an issue comment");
    assert_eq!(trailing_number(&comment.issue_url), Some(42));
}

/// A response missing the field must not be read as a comment on pull request
/// zero: the caller treats that as unclassified and attempts the reply anyway.
#[test]
fn a_response_without_a_parent_url_classifies_as_nothing() {
    let comment: ReviewCommentRef = serde_json::from_str(r#"{"id":1}"#).expect("no parent url");
    assert_eq!(trailing_number(&comment.pull_request_url), None);
}

#[test]
fn a_comment_on_this_pull_request_routes_by_the_space_it_came_from() {
    assert_eq!(
        resolve(Lookup::On(916), Lookup::Missing, 916),
        ReplyTarget::ReviewComment
    );
    assert_eq!(
        resolve(Lookup::Missing, Lookup::On(916), 916),
        ReplyTarget::ConversationComment
    );
}

/// The two id spaces number independently, so one integer can name a comment in
/// each. Reading only the review space would answer `Elsewhere(905)` here and
/// refuse a reply the caller can make.
#[test]
fn a_collision_is_settled_by_which_comment_is_on_the_named_pull_request() {
    assert_eq!(
        resolve(Lookup::On(905), Lookup::On(916), 916),
        ReplyTarget::ConversationComment
    );
    assert_eq!(
        resolve(Lookup::On(916), Lookup::On(905), 916),
        ReplyTarget::ReviewComment
    );
}

/// Both comments sit on the pull request asked about, and nothing says which
/// one the id was read from — so the caller picks rather than this code.
#[test]
fn a_collision_on_the_same_pull_request_is_ambiguous() {
    assert_eq!(
        resolve(Lookup::On(916), Lookup::On(916), 916),
        ReplyTarget::Ambiguous
    );
}

#[test]
fn a_comment_on_another_pull_request_names_that_one() {
    assert_eq!(
        resolve(Lookup::On(905), Lookup::Missing, 916),
        ReplyTarget::Elsewhere(905)
    );
    assert_eq!(
        resolve(Lookup::Missing, Lookup::On(1318), 916),
        ReplyTarget::Elsewhere(1318)
    );
    // Neither is on #916; the review space is named first because `pr reply`
    // is a thread reply and that is the space it addresses.
    assert_eq!(
        resolve(Lookup::On(905), Lookup::On(1318), 916),
        ReplyTarget::Elsewhere(905)
    );
}

/// Neither space holds the id — a deleted comment. The reply is attempted
/// anyway, so classifying must not turn a working call into an error.
#[test]
fn an_id_neither_space_knows_is_unclassified() {
    assert_eq!(
        resolve(Lookup::Missing, Lookup::Missing, 916),
        ReplyTarget::Unknown
    );
}

/// A lookup that failed is not an absent comment: the space it could not read
/// may hold the comment the caller meant, so its counterpart never routes the
/// reply on its own and the caller posts nothing.
#[test]
fn a_failed_lookup_leaves_the_id_unroutable_whatever_the_other_space_said() {
    for (review, issue) in [
        (Lookup::Failed, Lookup::On(916)),
        (Lookup::Failed, Lookup::On(1318)),
        (Lookup::Failed, Lookup::Missing),
        (Lookup::On(905), Lookup::Failed),
        (Lookup::Missing, Lookup::Failed),
        (Lookup::Failed, Lookup::Failed),
    ] {
        assert_eq!(resolve(review, issue, 916), ReplyTarget::Unreadable);
    }
}

/// A review comment carrying the id proves one exists, not that the id was read
/// off it — a conversation comment on the same pull request can carry it too.
/// With the space that would have said so unread, replying in the review thread
/// would answer a comment nobody addressed.
#[test]
fn a_review_comment_on_this_pull_request_does_not_settle_an_unread_counterpart() {
    assert_eq!(
        resolve(Lookup::On(916), Lookup::Failed, 916),
        ReplyTarget::Unreadable
    );
}

/// Only GitHub's own 404 means the space holds no such comment. An expired
/// token or a rate limit exits the same way and must not be read as one.
#[test]
fn only_a_404_says_the_comment_is_not_there() {
    assert!(is_not_found(
        "`gh api repos/o/r/pulls/comments/1` failed: gh: Not Found (HTTP 404)"
    ));
    assert!(!is_not_found(
        "`gh api repos/o/r/pulls/comments/1` failed: gh: Bad credentials (HTTP 401)"
    ));
    assert!(!is_not_found(
        "`gh api repos/o/r/pulls/comments/1` failed: gh: API rate limit exceeded (HTTP 403)"
    ));
    assert!(!is_not_found("failed to run gh: No such file or directory"));
}

/// A response whose parent URL names no pull request is a lookup that failed,
/// not a comment on pull request zero.
#[test]
fn an_unreadable_parent_url_is_a_failed_lookup() {
    assert_eq!(lookup(Ok(Some(String::new()))), Lookup::Failed);
    assert_eq!(
        lookup(Ok(Some(
            "https://api.github.com/repos/o/r/pulls/42".to_string()
        ))),
        Lookup::On(42)
    );
    assert_eq!(lookup(Ok(None)), Lookup::Missing);
    assert_eq!(
        lookup(Err("network unreachable".to_string())),
        Lookup::Failed
    );
}
