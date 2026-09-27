//! The external-id formats this Adapter persists.
//!
//! Written to `task_inbox.external_event_id` and a Binding's
//! `last_external_event_id`, read back by echo filtering and de-duplication;
//! changing a format doesn't migrate old rows, so extend through
//! `external_id_aliases` instead of editing a function here.
//!
//! One function per format, so the inbound/outbound spellings the echo filter
//! compares can't drift apart.

/// A Linear comment, inbound or outbound.
///
/// Written on both sides of the echo filter — `handle_comment_created` from the
/// webhook's comment id, `send_reply_for_binding` from the id `commentCreate`
/// returned — so they are one function the filter can compare by agreement.
pub(super) fn comment(comment_id: &str) -> String {
    format!("linear:comment:{comment_id}")
}

/// A follow-up prompt, named by the Linear activity carrying it.
///
/// Every other format here names the Linear resource it came from, and this one
/// does too.
pub(super) fn prompt(activity_id: &str) -> String {
    format!("linear:prompt:{activity_id}")
}

/// An issue state change, named by the issue and the semantic state-transition
/// identity the dispatcher derives for the delivery (see `ingress::dedup_key`).
///
/// The transition id is previous state, new state, and the issue's own
/// `updatedAt` when Linear includes it, falling back to `webhookTimestamp`
/// otherwise.
pub(super) fn issue_state(issue_id: &str, transition_id: &str) -> String {
    format!("linear:issue-state:{issue_id}:{transition_id}")
}

/// The inbound prompt of a task spawned from a mention or assignment.
pub(super) fn session_created(session_id: &str) -> String {
    format!("linear:session:created:{session_id}")
}

/// The inbound prompt of a task spawned from an issue entering a trigger state.
pub(super) fn issue_created(issue_id: &str) -> String {
    format!("linear:issue:created:{issue_id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The five formats, as literal strings.
    ///
    /// Asserted against literals, not against each other: a test comparing two
    /// functions would still pass if both changed together — precisely the
    /// change that breaks every row already in the database.
    #[test]
    fn every_format_is_pinned_to_its_literal() {
        assert_eq!(comment("cmt-1"), "linear:comment:cmt-1");
        assert_eq!(prompt("act-1"), "linear:prompt:act-1");
        assert_eq!(
            issue_state("issue-1", "st-old:st-new:2026-08-10T03:20:08.000Z"),
            "linear:issue-state:issue-1:st-old:st-new:2026-08-10T03:20:08.000Z"
        );
        assert_eq!(
            session_created("session-1"),
            "linear:session:created:session-1"
        );
        assert_eq!(issue_created("issue-1"), "linear:issue:created:issue-1");
    }

    /// No two formats collide on a shared prefix.
    ///
    /// They share a namespace compared as whole strings, so a format prefixing
    /// another would make one resource's id parse as another's if a parser is
    /// ever added — `linear:issue:created:` and `linear:issue-state:` are the
    /// near miss.
    #[test]
    fn no_format_is_a_prefix_of_another() {
        let ids = [
            comment("x"),
            prompt("x"),
            issue_state("x", "from:to:1"),
            session_created("x"),
            issue_created("x"),
        ];
        for (i, a) in ids.iter().enumerate() {
            for (j, b) in ids.iter().enumerate() {
                assert!(
                    i == j || !b.starts_with(a),
                    "{a} is a prefix of {b}: two resources would share one identity"
                );
            }
        }
    }
}
