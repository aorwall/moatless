//! Shared "Moatless task:" footer appended to outbound comments.
//!
//! Comments the agent posts to external providers (Linear, GitHub) end with a
//! link back to the moatless task so a human reading the external thread can
//! jump straight to it. The link text is the task's name, falling back to
//! `View task` when the task is unnamed.
//!
//! The footer doubles as the echo-filter source: when the provider echoes the
//! agent's own comment back via webhook, [`parse_task_id_from_body`] recovers
//! the authoring task id from the footer URL so we can drop only that task's
//! own echo (no hidden HTML marker required).

use crate::message::hooks::FOOTER_MARKER;
use crate::state::AppState;

/// Format the footer suffix from already-resolved inputs. Returns `None` when
/// `frontend_url` is empty (matches the PR-body footer behaviour in
/// `message::hooks`). The string includes the leading separator, e.g.
/// `"\n\n---\nMoatless task: [My task](https://app/tasks/abc)"`.
fn render_footer(frontend_url: &str, task_id: &str, name: Option<&str>) -> Option<String> {
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

/// Extract the moatless task id from a comment body that contains our footer
/// link `<frontend_url>/tasks/<id>`. Handles both the bare-URL and
/// `[name](url)` markdown forms. None when frontend_url is empty or absent.
pub(crate) fn parse_task_id_from_body(frontend_url: &str, body: &str) -> Option<String> {
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

/// Build the footer suffix appended to an outbound comment body, or `None` when
/// no frontend URL is configured. Loads the task name (best effort) to use as
/// the link text.
pub(crate) async fn task_comment_footer(state: &AppState, task_id: &str) -> Option<String> {
    let name = match state.tasks.find_by_id_internal(task_id).await {
        Ok(Some(task)) => task.name,
        Ok(None) => None,
        Err(e) => {
            tracing::warn!(task_id, error = %e, "Failed to load task name for comment footer");
            None
        }
    };
    render_footer(&state.config.auth.frontend_url, task_id, name.as_deref())
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
            Some("\n\n---\nMoatless task: [View task](https://app.example.com/tasks/abc)".to_string())
        );
        // Whitespace-only name is treated as no name.
        assert_eq!(
            render_footer("https://app.example.com", "abc", Some("   ")),
            Some("\n\n---\nMoatless task: [View task](https://app.example.com/tasks/abc)".to_string())
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
        let body =
            "Looks good.\n\n---\nMoatless task: [Fix the bug](https://app.example.com/tasks/abc-123)";
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
        // bare `[label](url)` footer with no `Moatless task:` prefix — the exact
        // shape that previously self-echoed back into the authoring task. The
        // parser must still recover the task id so routing skips that task.
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
}
