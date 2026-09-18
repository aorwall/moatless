//! Rendering several buffered events as one Task message.

use std::fmt::Write as _;

/// One buffered event, already rendered by its Adapter's inbound presentation.
pub(crate) struct Part {
    /// The handle the external system knows the writer by, where the event
    /// carried one.
    pub(crate) author: Option<String>,
    pub(crate) text: String,
}

/// Parts past this are left in the buffer for the next window, so one busy
/// subject cannot produce a message no agent can read.
pub(crate) const MAX_PARTS_PER_MESSAGE: usize = 25;

/// Join `parts` into the body of a single inbound turn.
///
/// A lone part renders exactly as it did before it was buffered, so turning
/// buffering on changes nothing for a subject that gets one event at a time.
/// Every part keeps its own author, because the message header the transcript
/// renders can only name one sender.
pub(crate) fn combine(mut parts: Vec<Part>) -> String {
    if parts.len() == 1 {
        return parts.swap_remove(0).text;
    }
    let total = parts.len();
    let mut out = format!(
        "{total} messages arrived on this thread within the buffering window, \
         oldest first. Handle all of them in this turn.\n"
    );
    for (index, part) in parts.iter().enumerate() {
        let from = part
            .author
            .as_deref()
            .map_or(String::new(), |author| format!(", from {author}"));
        let _ = write!(
            out,
            "\n--- message {} of {total}{from} ---\n\n{}\n",
            index + 1,
            part.text
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(author: Option<&str>, text: &str) -> Part {
        Part {
            author: author.map(ToString::to_string),
            text: text.to_string(),
        }
    }

    #[test]
    fn a_single_part_renders_as_its_own_text() {
        assert_eq!(combine(vec![part(Some("alice"), "just this")]), "just this");
    }

    #[test]
    fn several_parts_keep_their_order_authors_and_bodies() {
        let combined = combine(vec![
            part(Some("alice"), "first body"),
            part(None, "second body"),
        ]);
        assert!(combined.starts_with("2 messages arrived"), "{combined}");
        assert!(combined.contains("--- message 1 of 2, from alice ---"));
        assert!(combined.contains("--- message 2 of 2 ---"));
        let first = combined.find("first body").expect("first body is present");
        let second = combined
            .find("second body")
            .expect("second body is present");
        assert!(first < second, "parts must render oldest first");
    }
}
