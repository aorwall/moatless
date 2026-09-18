//! Shared "Moatless task:" footer appended to outbound comments.
//!
//! Comments posted to external providers (Linear, GitHub) end with a link back
//! to the moatless task. It doubles as the echo-filter source: when the
//! provider echoes the agent's own comment back via webhook,
//! [`parse_task_id_from_body`] recovers the authoring task id from the footer
//! URL so only that task's own echo is dropped, with no hidden HTML marker.
//!
//! This is the pure render/parse layer; the DAO-backed wrapper that loads a
//! task's name before rendering lives in the binding runtime
//! (`bindings::footer::task_comment_footer`).

/// Marker used to detect the footer was already appended, so a retry doesn't
/// re-append it.
///
/// Stable across runs; shared with the GitHub PR-body footer baked in by
/// `github_pr::api_routes::compose_footer`, which embeds the same marker.
pub const FOOTER_MARKER: &str = "Moatless task:";

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

/// Extract the moatless task id from a comment body containing the footer link
/// `<frontend_url>/tasks/<id>`.
///
/// Handles both the bare-URL and `[name](url)` markdown forms; `None` when
/// `frontend_url` is empty or absent.
pub fn parse_task_id_from_body(frontend_url: &str, body: &str) -> Option<String> {
    let base = frontend_url.trim_end_matches('/');
    if base.is_empty() {
        return None;
    }
    let needle = format!("{base}/tasks/");
    let start = body.find(&needle)? + needle.len();
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
        // Regression: the Linear direct-comment endpoint (triage flow) appends a
        // bare `[label](url)` footer with no `Moatless task:` prefix. The parser
        // must still recover the task id from this shape so routing skips that
        // task and avoids a self-echo.
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
        // Regression: the terminal Linear `response` activity posted by
        // `handle_agent_completed` must carry the footer so the inbound
        // `Comment.create` echo filter can recover the authoring task id and
        // drop the self-echo. Without it, the response re-enters the task as a
        // fresh comment and the agent loops forever. This asserts the exact
        // shape the completion handler emits (assistant text + footer) is
        // recognised as belonging to the originating task.
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
