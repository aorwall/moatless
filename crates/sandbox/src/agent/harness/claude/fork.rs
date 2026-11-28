//! Cutting a Claude transcript for a fork's first Turn.
//!
//! A fork resumes the parent's Session at one of its Turns, not its head, so
//! `--resume --fork-session` (which branches at the end) won't do. The pod
//! instead writes a truncated copy under a fresh session id and resumes that,
//! so the fork's Turns never land in the parent's transcript file.

use claude_agent_sdk::transcript_dir;
use tracing::warn;

/// Write a truncated copy of `source_session_id`'s transcript under a fresh
/// id, keeping every line through `through_message_uuid`; returns that id
/// for the harness to `--resume`.
///
/// Returns `None` when the source or the anchor line is missing, leaving
/// the caller to start the fork fresh rather than resume something unintended.
pub(super) async fn write_forked_transcript(
    cwd: &str,
    config_dir: &str,
    source_session_id: &str,
    through_message_uuid: &str,
) -> Option<String> {
    let dir = match transcript_dir(Some(cwd), Some(config_dir)) {
        Ok(dir) => dir,
        Err(error) => {
            warn!(%error, "fork: could not resolve the transcript dir; starting fresh");
            return None;
        }
    };
    let source = dir.join(format!("{source_session_id}.jsonl"));
    let body = match tokio::fs::read_to_string(&source).await {
        Ok(body) => body,
        Err(error) => {
            // NotFound is the isolated-fork case and is expected; log it thin.
            warn!(%error, path = %source.display(), "fork: source transcript unreadable; starting fresh");
            return None;
        }
    };

    let new_session_id = uuid::Uuid::new_v4().to_string();
    let cut = cut_transcript(&body, through_message_uuid, &new_session_id);
    if cut.is_empty() {
        warn!(
            through = %through_message_uuid,
            "fork: cut anchor not found in the transcript; starting fresh"
        );
        return None;
    }

    let dest = dir.join(format!("{new_session_id}.jsonl"));
    if let Err(error) = tokio::fs::write(&dest, cut).await {
        warn!(%error, path = %dest.display(), "fork: could not write the forked transcript; starting fresh");
        return None;
    }
    Some(new_session_id)
}

/// Keep `body`'s lines through the last one matching `through_uuid`,
/// rewriting `sessionId` to `session_id`.
///
/// Matches `message.id` first, then the line's own `uuid`. Cuts to the
/// *last* matching line, not the first, since one message spans up to
/// four lines. Empty string means no match; an unparseable line is dropped.
fn cut_transcript(body: &str, through_uuid: &str, session_id: &str) -> String {
    let parsed: Vec<serde_json::Value> = body
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .collect();

    let anchors = |value: &serde_json::Value| {
        let by_message_id = value
            .get("message")
            .and_then(|m| m.get("id"))
            .and_then(|id| id.as_str())
            == Some(through_uuid);
        by_message_id || value.get("uuid").and_then(|u| u.as_str()) == Some(through_uuid)
    };

    let Some(last) = parsed.iter().rposition(anchors) else {
        return String::new();
    };

    let mut out = String::new();
    for value in parsed.into_iter().take(last + 1) {
        let mut value = value;
        if let Some(obj) = value.as_object_mut()
            && obj.contains_key("sessionId")
        {
            obj.insert(
                "sessionId".to_string(),
                serde_json::Value::String(session_id.to_string()),
            );
        }
        out.push_str(&value.to_string());
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(uuid: &str, session: &str, kind: &str) -> String {
        serde_json::json!({ "uuid": uuid, "sessionId": session, "type": kind }).to_string()
    }

    /// An assistant line as the CLI writes it: a per-line `uuid` no database
    /// row holds, plus the `message.id` the Trajectory stores as its
    /// `harness_item_id`.
    fn assistant_line(uuid: &str, session: &str, message_id: &str) -> String {
        serde_json::json!({
            "uuid": uuid,
            "sessionId": session,
            "type": "assistant",
            "message": { "id": message_id, "role": "assistant" }
        })
        .to_string()
    }

    #[test]
    fn the_anchor_matches_the_message_id_the_database_stores() {
        // The anchor is what `anchors()` returns: an assistant row's uuid, which
        // is the Anthropic message id — never the CLI's per-line uuid.
        let body = [
            line("u1", "old", "user"),
            assistant_line("cli-a", "old", "msg_01A"),
            line("u2", "old", "user"),
            assistant_line("cli-b", "old", "msg_01B"),
        ]
        .join("\n");

        let cut = cut_transcript(&body, "msg_01A", "new");
        assert_eq!(cut.lines().count(), 2, "keeps through the msg_01A line");
        assert!(cut.contains("msg_01A"));
        assert!(
            !cut.contains("msg_01B"),
            "nothing after the anchor survives"
        );
    }

    #[test]
    fn a_message_spanning_several_lines_is_kept_whole() {
        // One assistant message occupies up to four consecutive transcript
        // lines. Cutting at the first would truncate the message being kept.
        let body = [
            line("u1", "old", "user"),
            assistant_line("cli-a", "old", "msg_01A"),
            assistant_line("cli-b", "old", "msg_01A"),
            assistant_line("cli-c", "old", "msg_01A"),
            line("u2", "old", "user"),
        ]
        .join("\n");

        let cut = cut_transcript(&body, "msg_01A", "new");
        assert_eq!(cut.lines().count(), 4, "all three anchor lines are kept");
        assert!(!cut.contains("\"u2\""), "the turn after the cut is dropped");
    }

    #[test]
    fn the_cut_keeps_through_the_anchor_and_rewrites_the_session() {
        let body = [
            line("a", "old", "user"),
            line("b", "old", "assistant"),
            line("c", "old", "user"),
        ]
        .join("\n");

        let cut = cut_transcript(&body, "b", "new");
        let kept: Vec<serde_json::Value> = cut
            .lines()
            .map(|l| serde_json::from_str(l).expect("valid json line"))
            .collect();

        assert_eq!(kept.len(), 2, "keeps a and b, drops c after the anchor");
        assert!(
            kept.iter().all(|v| v["sessionId"] == "new"),
            "every kept line carries the fork's id, not the parent's"
        );
        assert_eq!(kept[1]["uuid"], "b");
    }

    #[test]
    fn a_missing_anchor_cuts_nothing() {
        let body = [line("a", "old", "user"), line("b", "old", "assistant")].join("\n");
        // Never copy the whole session because the anchor was not found.
        assert_eq!(cut_transcript(&body, "z", "new"), "");
    }

    #[test]
    fn an_unparseable_line_is_dropped_not_carried() {
        let body = [
            line("a", "old", "user"),
            "this is not json".to_string(),
            line("b", "old", "assistant"),
        ]
        .join("\n");

        let cut = cut_transcript(&body, "b", "new");
        assert_eq!(cut.lines().count(), 2, "the junk line is gone");
        assert!(!cut.contains("not json"));
        assert!(!cut.contains("\"old\""), "no parent id survives the cut");
    }

    #[test]
    fn a_line_without_a_session_id_is_kept_untouched() {
        let body = [
            serde_json::json!({ "uuid": "a", "type": "summary" }).to_string(),
            line("b", "old", "assistant"),
        ]
        .join("\n");

        let cut = cut_transcript(&body, "b", "new");
        let first: serde_json::Value = serde_json::from_str(cut.lines().next().unwrap()).unwrap();
        assert!(first.get("sessionId").is_none(), "none is invented");
        assert_eq!(first["uuid"], "a");
    }

    #[test]
    fn the_anchor_on_the_first_line_keeps_only_it() {
        let body = [line("a", "old", "user"), line("b", "old", "assistant")].join("\n");
        assert_eq!(cut_transcript(&body, "a", "new").lines().count(), 1);
    }
}
