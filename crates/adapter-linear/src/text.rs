//! Everything the Adapter renders for a person to read.
//!
//! The inbound prompt (read by the agent) stays deliberately *minimal* — the
//! issue body, comments, and Linear's `promptContext` are omitted since the
//! agent reads the live ticket via the `moat` CLI instead of a stale snapshot.
//! The attachment title (read by a Linear-side person) names the Moatless task
//! the pill points at.
//!
//! Kept together as pure formatting with no policy; the two inbound prompts
//! share [`issue_read_block`] so they can't drift apart.

use std::fmt::Write as _;

use moatless_core::config::Config;

use super::webhook_payloads::{AgentSession, IssuePayload};

/// The frontend URL of a Moatless task, as linked from Linear.
pub(super) fn task_url(config: &Config, task_id: &str) -> String {
    let base = config.auth.frontend_url.trim_end_matches('/');
    format!("{base}/tasks/{task_id}")
}

pub(super) fn build_inbound_text(session: &AgentSession) -> String {
    // Intentionally minimal: the issue description, comments, and Linear's
    // pre-built promptContext are NOT inlined — they go stale as the ticket
    // evolves. The agent reads the live ticket via the `moat` CLI instead.
    match session.issue.as_ref() {
        Some(issue) => issue_read_block(&issue.identifier, &issue.title, issue.url.as_deref()),
        None => String::new(),
    }
}

/// Inbound prompt for a task spawned from an `Issue.update` status entry.
/// Mirrors [`build_inbound_text`] — the agent reads the live ticket rather than
/// receiving an inlined (and quickly-stale) snapshot.
pub(super) fn build_issue_inbound_text(issue: &IssuePayload) -> String {
    issue_read_block(&issue.identifier, &issue.title, issue.url.as_deref())
}

/// Shared issue-link + "read the live ticket via `moat`" instruction block used
/// by both the session- and issue-initiated creation paths. Keeping it in one
/// place prevents the two prompts from drifting apart.
fn issue_read_block(identifier: &str, title: &str, url: Option<&str>) -> String {
    let mut text = String::new();
    let _ = write!(text, "## Linear issue {identifier} — {title}");
    if let Some(url) = url.filter(|s| !s.is_empty()) {
        let _ = write!(text, "\n\n{url}");
    }
    let id = identifier;
    let _ = write!(
        text,
        "\n\nBefore working on this task, read the Linear issue, all of its \
         comments, and its linked PRs and agent tasks using the `moat` CLI:\n\n\
         - Issue details, description and comments: \
         `moat linear issue get {id} --json`\n\
         - Linked PRs and related agent tasks: \
         `moat linear graphql 'query {{ issue(id: \"{id}\") {{ \
         attachments {{ nodes {{ title url sourceType }} }} }} }}'`\n\n\
         Read people off `issue get`, not off a raw `graphql` query: it names \
         each assignee, delegate, creator and comment author with a \
         `moatlessLogin` alongside Linear's own fields, and that login is the \
         only one of those names you can address a person by elsewhere in \
         Moatless. A Linear `displayName` is not a login."
    );
    text
}

/// Label prefix for the Linear ↔ Moatless link when the Linear side is the
/// parent (Moatless task spawned from a `@mention` / assignment).
pub(super) const ATTACHMENT_TITLE_CHILD: &str = "Moatless task";

/// Label prefix for the Linear ↔ Moatless link when the Moatless side is the
/// parent (Linear issue created by the CLI or `adapter_send` on behalf of a
/// task).
pub(super) const ATTACHMENT_TITLE_PARENT: &str = "Moatless parent task";

pub(super) fn format_attachment_title(prefix: &str, name: Option<&str>) -> String {
    match name.map(str::trim).filter(|s| !s.is_empty()) {
        Some(n) => format!("{prefix}: {n}"),
        None => prefix.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::webhook_payloads::{AgentSessionComment, AgentSessionIssue, LinearActor};

    fn issue() -> AgentSessionIssue {
        AgentSessionIssue {
            id: "issue-1".into(),
            identifier: "SP-1".into(),
            title: "Bug".into(),
            description: Some("Long description here.".into()),
            url: Some("https://linear.app/acme/issue/SP-1".into()),
            team_id: Some("team-1".into()),
            team: None,
            state: None,
        }
    }

    fn session_with(
        comment: Option<AgentSessionComment>,
        prompt_context: Option<String>,
    ) -> AgentSession {
        AgentSession {
            id: "session-1".into(),
            issue: Some(issue()),
            comment,
            creator: None,
            prompt_context,
        }
    }

    fn user_comment(id: &str, body: &str, user_name: &str) -> AgentSessionComment {
        AgentSessionComment {
            id: id.into(),
            body: body.into(),
            user: Some(LinearActor {
                id: "user-1".into(),
                name: user_name.into(),
                email: None,
            }),
        }
    }

    #[test]
    fn build_inbound_text_links_issue_with_read_instructions() {
        let session = session_with(None, None);
        let text = build_inbound_text(&session);
        assert!(text.contains("## Linear issue SP-1 — Bug"));
        assert!(text.contains("https://linear.app/acme/issue/SP-1"));
        assert!(text.contains(
            "read the Linear issue, all of its comments, and its linked PRs and agent tasks"
        ));
        assert!(text.contains("moat linear issue get SP-1 --json"));
        assert!(text.contains(
            "moat linear graphql 'query { issue(id: \"SP-1\") { \
             attachments { nodes { title url sourceType } } } }'"
        ));
        // The issue description is not inlined — the agent reads the live ticket.
        assert!(!text.contains("Long description here."));
    }

    /// Comments come from `issue get`, which annotates each author with a
    /// `moatlessLogin`. Sending the agent to a raw `graphql` query for them
    /// hands it a `displayName` it cannot address anyone by, which is how a
    /// Linear name once reached a Slack mention.
    #[test]
    fn the_read_block_sources_people_from_the_annotated_command() {
        let text = build_inbound_text(&session_with(None, None));
        assert!(
            !text.contains("user { displayName }"),
            "no raw query for comment authors"
        );
        assert!(text.contains("moatlessLogin"));
        assert!(text.contains("A Linear `displayName` is not a login."));
    }

    #[test]
    fn build_issue_inbound_text_matches_session_variant() {
        use crate::webhook_payloads::IssuePayload;
        let issue = IssuePayload {
            id: "issue-1".into(),
            identifier: "SP-1".into(),
            title: "Bug".into(),
            url: Some("https://linear.app/acme/issue/SP-1".into()),
            team_id: Some("team-1".into()),
            team: None,
            project_id: Some("proj-1".into()),
            project: None,
            updated_at: None,
            state: None,
            label_ids: Vec::new(),
            labels: Vec::new(),
        };
        let from_issue = build_issue_inbound_text(&issue);
        // Identical to the session-initiated prompt for the same issue identity.
        let from_session = build_inbound_text(&session_with(None, None));
        assert_eq!(from_issue, from_session);
        assert!(from_issue.contains("## Linear issue SP-1 — Bug"));
        assert!(from_issue.contains("moat linear issue get SP-1 --json"));
    }

    #[test]
    fn build_inbound_text_ignores_prompt_context() {
        let session = session_with(None, Some("<context>stale snapshot</context>".into()));
        let text = build_inbound_text(&session);
        assert!(!text.contains("stale snapshot"));
        assert!(text.contains("moat linear issue get SP-1 --json"));
    }

    #[test]
    fn build_inbound_text_never_inlines_comment_bodies() {
        for comment in [
            AgentSessionComment {
                id: "sys-1".into(),
                body: "This thread is for an agent session with moatless.".into(),
                user: None,
            },
            user_comment("c-1", "please look at this", "Alice"),
        ] {
            let session = session_with(Some(comment.clone()), None);
            let text = build_inbound_text(&session);
            assert!(text.contains("## Linear issue SP-1 — Bug"));
            assert!(!text.contains(&comment.body));
            assert!(!text.contains("just mentioned the agent"));
        }
    }

    #[test]
    fn build_inbound_text_handles_missing_issue() {
        let session = AgentSession {
            id: "session-1".into(),
            issue: None,
            comment: Some(user_comment("c-1", "hey", "Alice")),
            creator: None,
            prompt_context: None,
        };
        let text = build_inbound_text(&session);
        assert!(text.is_empty());
    }

    #[test]
    pub(super) fn format_attachment_title_with_name() {
        assert_eq!(
            format_attachment_title(ATTACHMENT_TITLE_CHILD, Some("Fix login bug")),
            "Moatless task: Fix login bug"
        );
        assert_eq!(
            format_attachment_title(ATTACHMENT_TITLE_PARENT, Some("Fix login bug")),
            "Moatless parent task: Fix login bug"
        );
    }

    #[test]
    pub(super) fn format_attachment_title_falls_back_when_name_missing() {
        assert_eq!(
            format_attachment_title(ATTACHMENT_TITLE_CHILD, None),
            "Moatless task"
        );
        assert_eq!(
            format_attachment_title(ATTACHMENT_TITLE_PARENT, None),
            "Moatless parent task"
        );
    }

    #[test]
    pub(super) fn format_attachment_title_treats_whitespace_only_as_empty() {
        assert_eq!(
            format_attachment_title(ATTACHMENT_TITLE_CHILD, Some("   ")),
            "Moatless task"
        );
    }

    #[test]
    pub(super) fn format_attachment_title_trims_surrounding_whitespace() {
        assert_eq!(
            format_attachment_title(ATTACHMENT_TITLE_CHILD, Some("  Hi  ")),
            "Moatless task: Hi"
        );
    }
}
