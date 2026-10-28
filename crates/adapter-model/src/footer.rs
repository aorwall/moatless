//! Shared "Moatless task:" footer appended to outbound comments.
//!
//! Comments posted to external providers (Linear, GitHub) end with a link back
//! to the moatless task. It doubles as the echo-filter source: when the
//! provider echoes the agent's own comment back via webhook,
//! [`parse_task_id_from_body`] recovers the authoring task id from the footer
//! URL so only that task's own echo is dropped, with no hidden HTML marker.
//!
//! This is the pure render/parse layer; the DAO-backed wrapper lives in
//! `bindings::footer::task_comment_footer`.

/// Detects the footer was already appended, so a retry doesn't re-append it.
pub use moatless_types::adapter::FOOTER_MARKER;

/// Format the footer suffix from already-resolved inputs.
///
/// Returns `None` when `frontend_url` is empty, matching the GitHub PR-body
/// footer. Includes the leading separator, e.g. `"\n\n---\nMoatless task: [My
/// task](https://app/tasks/abc)"`.
pub fn render_footer(frontend_url: &str, task_id: &str, name: Option<&str>) -> Option<String> {
    let base = frontend_url.trim_end_matches('/');
    if base.is_empty() {
        return None;
    }
    let url = format!("{base}/tasks/{task_id}");
    let label = name
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("View task");
    Some(format!("\n\n---\n{FOOTER_MARKER} [{label}]({url})"))
}

/// Extract the moatless task id from the last `<frontend_url>/tasks/<id>` link
/// in a comment body, bare or `[name](url)`; `None` when `frontend_url` is empty.
///
/// The last link rather than the first, deliberately: the footer ends the body,
/// and a reply citing another task would otherwise echo back to its author.
pub fn parse_task_id_from_body(frontend_url: &str, body: &str) -> Option<String> {
    let base = frontend_url.trim_end_matches('/');
    if base.is_empty() {
        return None;
    }
    let needle = format!("{base}/tasks/");
    let start = body.rfind(&needle)? + needle.len();
    let id: String = body[start..]
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    (!id.is_empty()).then_some(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_task_name_as_link_text() {
        assert_eq!(
            render_footer("https://app.example.com", "abc", Some("Fix the bug")),
            Some(
                "\n\n---\nMoatless task: [Fix the bug](https://app.example.com/tasks/abc)"
                    .to_string()
            )
        );
    }

    #[test]
    fn falls_back_to_view_task_without_name() {
        assert_eq!(
            render_footer("https://app.example.com/", "abc", None),
            Some(
                "\n\n---\nMoatless task: [View task](https://app.example.com/tasks/abc)"
                    .to_string()
            )
        );
        // Whitespace-only name is treated as no name.
        assert_eq!(
            render_footer("https://app.example.com", "abc", Some("   ")),
            Some(
                "\n\n---\nMoatless task: [View task](https://app.example.com/tasks/abc)"
                    .to_string()
            )
        );
    }

    #[test]
    fn trims_trailing_slash_on_base() {
        let footer = render_footer("https://app.example.com///", "x", Some("n")).unwrap();
        assert!(footer.contains("https://app.example.com/tasks/x"));
    }

    #[test]
    fn none_when_frontend_url_empty() {
        assert_eq!(render_footer("", "abc", Some("n")), None);
    }

    #[test]
    fn parses_task_id_from_markdown_link_form() {
        let body = "Looks good.\n\n---\nMoatless task: [Fix the bug](https://app.example.com/tasks/abc-123)";
        assert_eq!(
            parse_task_id_from_body("https://app.example.com", body),
            Some("abc-123".to_string())
        );
        // Trailing slash on the configured base is normalised away.
        assert_eq!(
            parse_task_id_from_body("https://app.example.com/", body),
            Some("abc-123".to_string())
        );
    }

    #[test]
    fn parses_task_id_from_bare_url_form() {
        let body = "see https://app.example.com/tasks/task_42 for details";
        assert_eq!(
            parse_task_id_from_body("https://app.example.com", body),
            Some("task_42".to_string())
        );
    }

    #[test]
    fn parses_task_id_from_production_direct_comment_footer() {
        // Linear's triage endpoint appends a bare `[label](url)` footer with no
        // `Moatless task:` prefix; the parser must still recover the task id,
        // or routing fails to skip the self-echo.
        let body = "Triage summary…\n\n---\n[Triage E2E run 27608948265]\
            (https://moatless.xyz/tasks/731bd010-0f54-4ec0-a099-b94b1c16c9ca)";
        assert_eq!(
            parse_task_id_from_body("https://moatless.xyz", body),
            Some("731bd010-0f54-4ec0-a099-b94b1c16c9ca".to_string())
        );
    }

    #[test]
    fn parse_none_when_frontend_url_empty_or_no_match() {
        // Empty configured frontend_url disables link-based parsing.
        assert_eq!(
            parse_task_id_from_body("", "https://app.example.com/tasks/abc"),
            None
        );
        // No footer URL present in the body.
        assert_eq!(
            parse_task_id_from_body("https://app.example.com", "just a human comment"),
            None
        );
        // URL present but with an empty id segment.
        assert_eq!(
            parse_task_id_from_body("https://app.example.com", "https://app.example.com/tasks/"),
            None
        );
    }

    #[test]
    fn a_reply_citing_another_task_is_attributed_to_its_footer() {
        // Linear echoes the body with its links rewritten to `[url](<url>)`.
        let body = "The other task ([https://moatless.xyz/tasks/fc602fec]\
            (<https://moatless.xyz/tasks/fc602fec>)) holds the claim.\n\n---\n\
            Moatless task: [SP-4145](<https://moatless.xyz/tasks/6852f3a4>)";
        assert_eq!(
            parse_task_id_from_body("https://moatless.xyz", body),
            Some("6852f3a4".to_string())
        );
    }

    #[test]
    fn render_and_parse_round_trip() {
        let footer = render_footer("https://app.example.com", "abc-123", Some("Fix the bug"))
            .expect("footer rendered");
        let body = format!("My reply.{footer}");
        assert_eq!(
            parse_task_id_from_body("https://app.example.com", &body),
            Some("abc-123".to_string())
        );
    }

    #[test]
    fn completion_response_body_is_echo_filterable() {
        // The terminal `response` activity from `handle_agent_completed` must
        // carry the footer: without it, the inbound echo filter can't match it
        // and the response re-enters the task, looping the agent forever.
        let frontend = "https://moatless.xyz";
        let task_id = "9a7c77ab-5e43-45b5-9315-955802f9b3b2";
        let footer = render_footer(frontend, task_id, Some("SP-2360")).expect("footer rendered");
        let response_body = format!("The task is complete.{footer}");
        assert_eq!(
            parse_task_id_from_body(frontend, &response_body),
            Some(task_id.to_string()),
            "completion response body must be attributable to its authoring task"
        );
    }
}
