//! Context for questions beside an active main task.

const MAX_CONTEXT_CHARS: usize = 8_000;
const MAX_ITEM_CHARS: usize = 240;
const TRUNCATED: &str = "- Earlier items truncated.\n";

pub(crate) fn digest(parent_id: &str, items: &[(String, String)]) -> String {
    let mut digest = String::new();
    let mut remaining = MAX_CONTEXT_CHARS
        .saturating_sub(section(parent_id, "").chars().count() + TRUNCATED.chars().count());
    for (author, text) in items {
        let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let item: String = format!("{author}: {compact}")
            .chars()
            .take(MAX_ITEM_CHARS)
            .collect();
        let line = format!("- {item}\n");
        let length = line.chars().count();
        if length > remaining {
            digest.push_str(TRUNCATED);
            break;
        }
        remaining -= length;
        digest.push_str(&line);
    }
    digest
}

pub(crate) fn section(parent_id: &str, context: &str) -> String {
    format!(
        "# Side Chat\n\nYou are in a side chat beside the main thread. The main thread may still be running in the same worktree. Answer the user's questions; do not continue or redo the main thread's task.\n\nMain thread: `{parent_id}`. The inherited conversation stops at the fork point.\n\n{context}\nRun `moat messages list {parent_id} --full` to see the main thread's live state."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_is_compact_bounded_and_keeps_newest_items_first() {
        let mut items = vec![("user".into(), "newest\n   question".into())];
        items.extend((0..80).map(|_| ("agent".into(), "é".repeat(300))));
        let text = digest("parent", &items);
        assert!(text.starts_with("- user: newest question\n"));
        assert!(text.ends_with(TRUNCATED));
        assert!(text.chars().count() <= MAX_CONTEXT_CHARS);
        assert!(
            text.lines()
                .all(|line| line.chars().count() <= MAX_ITEM_CHARS + 2)
        );
        let prompt = section("parent", &text);
        assert!(prompt.chars().count() <= MAX_CONTEXT_CHARS);
        assert!(prompt.contains("# Side Chat"));
        assert!(prompt.contains("do not continue or redo the main thread's task"));
        assert!(prompt.contains("moat messages list parent --full"));
        assert!(!prompt.contains("moat send"));
    }
}
