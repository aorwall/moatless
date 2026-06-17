//! Shared agent-comment marker used by outbound comments across providers.
//!
//! Outbound comment bodies are tagged with the source task id
//! (`<!-- moatless-agent:TASK_ID -->`) so that when the provider echoes the
//! comment back via webhook we can drop only the authoring task's own echo,
//! while still fanning the comment out to other tasks bound to the same
//! subject. The marker is an HTML comment so it stays hidden in rendered
//! markdown on GitHub and Linear.

/// Agent comment marker prefix.
const MARKER_PREFIX: &str = "<!-- moatless-agent";

/// Build the outbound agent marker for a given task id.
pub(crate) fn build(task_id: &str) -> String {
    format!("<!-- moatless-agent:{task_id} -->")
}

/// Classification of a leading agent marker on a comment body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AgentMarker {
    /// Legacy bare marker `<!-- moatless-agent -->` — no task id embedded.
    /// Predates per-task tagging; still ignored on inbound for backward compat.
    Bare,
    /// New tagged marker `<!-- moatless-agent:TASK_ID -->`.
    Tagged(String),
}

/// Parse the leading agent marker (if any) from a comment body.
///
/// Matches only at `body.trim_start()`. Returns `None` when the body does not
/// begin with the marker prefix; `Some(Bare)` for legacy bare markers or any
/// malformed tagged form (e.g. empty id) — these are treated as
/// "agent-authored, source unknown" and dropped on inbound; `Some(Tagged(id))`
/// when an id is present.
pub(crate) fn parse(body: &str) -> Option<AgentMarker> {
    let trimmed = body.trim_start();
    let rest = trimmed.strip_prefix(MARKER_PREFIX)?;
    // `rest` now begins with either ` -->` (bare) or `:<id> -->` (tagged).
    if let Some(after_colon) = rest.strip_prefix(':') {
        if let Some(end) = after_colon.find("-->") {
            let id = after_colon[..end].trim();
            if id.is_empty() {
                return Some(AgentMarker::Bare);
            }
            return Some(AgentMarker::Tagged(id.to_string()));
        }
        return Some(AgentMarker::Bare);
    }
    if rest.trim_start().starts_with("-->") {
        return Some(AgentMarker::Bare);
    }
    None
}

/// Strip a leading agent marker line from a comment body, returning the
/// remaining text. Used on inbound so the agent never sees the marker. When no
/// marker is present the body is returned unchanged (trimmed of a leading
/// newline only when a marker was removed).
pub(crate) fn strip(body: &str) -> &str {
    let Some(rest) = body.trim_start().strip_prefix(MARKER_PREFIX) else {
        return body;
    };
    let Some(end) = rest.find("-->") else {
        return body;
    };
    // `after` is a suffix of `body`; skip the closing `-->` and one newline.
    let after = &rest[end + 3..];
    after.strip_prefix('\n').unwrap_or(after)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_classifies_bodies() {
        assert!(parse("hello world").is_none());
        assert!(parse("").is_none());

        assert_eq!(parse("<!-- moatless-agent -->\nbody"), Some(AgentMarker::Bare));
        assert_eq!(
            parse("   \n<!-- moatless-agent -->\nbody"),
            Some(AgentMarker::Bare)
        );
        assert_eq!(
            parse("<!-- moatless-agent:task-abc -->\nbody"),
            Some(AgentMarker::Tagged("task-abc".to_string()))
        );
        assert_eq!(
            parse("<!-- moatless-agent: task-abc  -->\nbody"),
            Some(AgentMarker::Tagged("task-abc".to_string()))
        );
        assert_eq!(parse("<!-- moatless-agent: -->\nbody"), Some(AgentMarker::Bare));
        // Marker must lead the body.
        assert!(parse("real comment\n<!-- moatless-agent:x -->").is_none());
    }

    #[test]
    fn build_round_trips_through_parse() {
        let body = format!("{}\nhello", build("task-42"));
        assert_eq!(parse(&body), Some(AgentMarker::Tagged("task-42".to_string())));
    }

    #[test]
    fn strip_removes_leading_marker_line() {
        let body = format!("{}\nhello world", build("task-42"));
        assert_eq!(strip(&body), "hello world");
        // No marker → unchanged.
        assert_eq!(strip("just a comment"), "just a comment");
        // Bare marker stripped too.
        assert_eq!(strip("<!-- moatless-agent -->\nhi"), "hi");
    }
}
