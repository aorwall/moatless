//! What a Task made on GitHub, as the record [`OwnEvents`] files it under.
//!
//! The one place outside this crate's routing that names a `github:` id: the
//! git proxy and the GitHub API proxy describe what passed through them here.
//!
//! [`OwnEvents`]: moatless_loops::bindings::own_events::OwnEvents

use moatless_ports::task::ExternalEvent;

use super::external_id;
use super::routing::ADAPTER_LABEL;

/// A thing a Task made on one repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Made {
    /// A commit the Task put on a branch; its `synchronize` aliases onto it.
    Push(String),
    IssueComment(u64),
    ReviewComment(u64),
    Review(u64),
    /// A pull request field the Task set, as [`Made::pull_request_edit`]
    /// digests it; its `edited` aliases onto it.
    PullRequestEdit(String),
    /// A conversation comment body the Task wrote over, as
    /// [`Made::issue_comment_edit`] digests it.
    IssueCommentEdit(String),
}

impl Made {
    /// The Task set the pull request's `field` (`title`, `body` or `base`, the
    /// base branch's name) to `value`.
    pub fn pull_request_edit(field: &str, value: &str) -> Self {
        Self::PullRequestEdit(external_id::edit_digest(field, value))
    }

    /// The Task edited a conversation comment to read `body`.
    pub fn issue_comment_edit(body: &str) -> Self {
        Self::IssueCommentEdit(external_id::edit_digest("comment", body))
    }
}

/// The record `made` on `owner/repo` is filed under.
pub fn event(owner: &str, repo: &str, made: &Made) -> ExternalEvent {
    let external_id = match made {
        Made::Push(sha) => external_id::push(owner, repo, sha),
        Made::IssueComment(id) => external_id::issue_comment(*id),
        Made::ReviewComment(id) => external_id::review_comment(*id),
        Made::Review(id) => external_id::review(*id),
        Made::PullRequestEdit(digest) => external_id::pr_edit(owner, repo, digest),
        Made::IssueCommentEdit(digest) => external_id::issue_comment_edit(owner, repo, digest),
    };
    on_repo(external_id, owner, repo)
}

/// An already-encoded `external_id` on `owner/repo`.
pub(crate) fn on_repo(external_id: String, owner: &str, repo: &str) -> ExternalEvent {
    ExternalEvent {
        external_id,
        provider: ADAPTER_LABEL.to_string(),
        source_name: Some(format!("{owner}/{repo}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_push_is_filed_where_its_synchronize_looks_for_the_maker() {
        let pushed = event("acme", "widgets", &Made::Push("be1d882".into()));

        assert_eq!(pushed.source_name.as_deref(), Some("acme/widgets"));
        assert!(
            external_id::aliases("github:pr:synchronize:acme/widgets#7:be1d882")
                .contains(&pushed.external_id)
        );
    }

    #[test]
    fn an_edit_is_filed_where_its_edited_looks_for_the_maker() {
        let made = Made::pull_request_edit("title", "feat: x");
        let edited = event("acme", "widgets", &made);
        let Made::PullRequestEdit(digest) = made else {
            unreachable!()
        };

        assert!(
            external_id::aliases(&format!("github:pr:edited:acme/widgets#7:{digest}"))
                .contains(&edited.external_id)
        );
    }

    #[test]
    fn a_comment_edit_is_filed_where_its_edited_comment_looks_for_the_maker() {
        let made = Made::issue_comment_edit("Plan: 1 to add");
        let filed = event("acme", "widgets", &made);
        let Made::IssueCommentEdit(digest) = made else {
            unreachable!()
        };

        assert!(
            external_id::aliases(&format!("github:ic-edited:acme/widgets#7:42:{digest}"))
                .contains(&filed.external_id)
        );
    }
}
