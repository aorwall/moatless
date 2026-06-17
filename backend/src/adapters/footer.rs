//! Shared "view task" footer appended to outbound comments.
//!
//! Comments the agent posts to external providers (Linear, GitHub) end with a
//! link back to the moatless task so a human reading the external thread can
//! jump straight to it. The link text is the task's name, falling back to
//! `View task` when the task is unnamed.

use crate::state::AppState;

/// Format the footer suffix from already-resolved inputs. Returns `None` when
/// `frontend_url` is empty (matches the PR-body footer behaviour in
/// `message::hooks`). The string includes the leading separator, e.g.
/// `"\n\n---\n[My task](https://app/tasks/abc)"`.
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
    Some(format!("\n\n---\n[{label}]({url})"))
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
            Some("\n\n---\n[Fix the bug](https://app.example.com/tasks/abc)".to_string())
        );
    }

    #[test]
    fn falls_back_to_view_task_without_name() {
        assert_eq!(
            render_footer("https://app.example.com/", "abc", None),
            Some("\n\n---\n[View task](https://app.example.com/tasks/abc)".to_string())
        );
        // Whitespace-only name is treated as no name.
        assert_eq!(
            render_footer("https://app.example.com", "abc", Some("   ")),
            Some("\n\n---\n[View task](https://app.example.com/tasks/abc)".to_string())
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
}
