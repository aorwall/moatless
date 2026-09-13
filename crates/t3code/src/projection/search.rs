//! A thread search match, with the snippet the client highlights.

use serde_json::{Value, json};

use super::scalar::nullable_instant;
use crate::dao::ConversationMatch;

/// The conversations a search matched, in the shape `searchThreads` returns.
///
/// A match with no Workspace is dropped: the contract's match carries a project
/// id the client routes on, and those are the same older, hand-written rows the
/// shell listing already excludes.
pub(crate) fn thread_matches(matches: &[ConversationMatch], query: &str) -> Vec<Value> {
    matches
        .iter()
        .filter_map(|matched| {
            let project_id = matched.workspace_id.as_deref()?;
            Some(json!({
                "threadId": matched.task_id,
                "projectId": project_id,
                "source": if matched.from_user { "user" } else { "assistant" },
                "snippet": snippet(&matched.matched_text, query),
                "messageCreatedAt": nullable_instant(matched.matched_at),
            }))
        })
        .collect()
}

/// The most one search snippet carries, and how much of it is text.
///
/// The contract caps the field at 240 (`orchestration.ts:1412`); the body is
/// four short of it to leave room for the two ellipses a windowed snippet
/// carries.
pub(super) const SNIPPET_MAX: usize = 240;

const SNIPPET_BODY: usize = 236;

/// Enough of the matched text to recognise the conversation by.
///
/// Whitespace is collapsed first since a Message's text wraps across lines;
/// anything still too long is windowed around the match rather than cut from
/// the start, since a snippet without the match is a row nobody can place.
/// Counted in characters, not bytes.
pub(super) fn snippet(text: &str, query: &str) -> String {
    let normalized: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let characters: Vec<char> = normalized.chars().collect();
    if characters.len() <= SNIPPET_MAX {
        return normalized;
    }

    let folded = normalized.to_lowercase();
    // Byte offset of the match, then how many characters precede it — the two
    // are not the same number the moment anything is non-ASCII.
    let matched_at = folded
        .find(&query.trim().to_lowercase())
        .map_or(0, |offset| folded[..offset].chars().count());
    let start = matched_at
        .saturating_sub(72)
        .min(characters.len() - SNIPPET_BODY);
    let end = (start + SNIPPET_BODY).min(characters.len());

    let mut snippet = String::new();
    if start > 0 {
        snippet.push('…');
    }
    snippet.extend(&characters[start..end]);
    if end < characters.len() {
        snippet.push('…');
    }
    snippet
}
