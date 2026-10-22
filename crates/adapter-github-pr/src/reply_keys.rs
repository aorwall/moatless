//! Reply keys: how a GitHub event names the writing it answers, matched
//! against the keys a task's own writing recorded on its Binding.
//!
//! A review comment answers its thread, keyed by the thread's root comment.
//! The PR conversation has no threads, so a conversation comment answers a
//! task's comment when it links to it (`#issuecomment-<id>`), or when its
//! sender is someone that task @mentioned in a conversation comment on the PR.

/// The `InboundMessage::adapter_context` key listing the reply keys a reply
/// to this event would carry, read when a task is named as its author.
pub(super) const OPENS_KEY: &str = "opens_reply_keys";

/// The review thread whose root comment is `root_comment_id`.
pub(super) fn review_thread(root_comment_id: u64) -> String {
    format!("github:review_thread:{root_comment_id}")
}

/// The conversation comment `comment_id`.
pub(super) fn conversation_comment(comment_id: u64) -> String {
    super::external_id::issue_comment(comment_id)
}

/// The prefix every [`addressed`] key starts with.
const ADDRESSED_PREFIX: &str = "github:addressed:";

/// A person a task addressed by login. GitHub logins are case-insensitive,
/// and an App is mentioned as `@name` but sends as `name[bot]`.
pub(super) fn addressed(login: &str) -> String {
    let login = login.strip_suffix("[bot]").unwrap_or(login);
    format!("{ADDRESSED_PREFIX}{}", login.to_ascii_lowercase())
}

/// Whether `key` matches by sender rather than by the comment answered.
pub(super) fn is_addressed(key: &str) -> bool {
    key.starts_with(ADDRESSED_PREFIX)
}

/// What a review comment answers: its thread, when it replies in one.
pub(super) fn answered_by_review_comment(in_reply_to_id: Option<u64>) -> Vec<String> {
    in_reply_to_id.map(review_thread).into_iter().collect()
}

/// What a review comment lets others answer: the thread it starts or joins.
pub(super) fn opened_by_review_comment(
    comment_id: u64,
    in_reply_to_id: Option<u64>,
) -> Vec<String> {
    vec![review_thread(in_reply_to_id.unwrap_or(comment_id))]
}

/// What a conversation comment answers: each comment it links to, and every
/// conversation comment its sender was addressed in.
pub(super) fn answered_by_conversation_comment(body: &str, sender_login: &str) -> Vec<String> {
    let mut keys: Vec<String> = linked_comment_ids(body)
        .into_iter()
        .map(conversation_comment)
        .collect();
    if !sender_login.is_empty() {
        keys.push(addressed(sender_login));
    }
    keys
}

/// What a conversation comment lets others answer: the comment itself, and a
/// reply from each login it @mentions.
pub(super) fn opened_by_conversation_comment(comment_id: u64, body: &str) -> Vec<String> {
    let mut keys = vec![conversation_comment(comment_id)];
    keys.extend(mentions(body).iter().map(|login| addressed(login)));
    keys
}

/// The comment ids `body` links to as `…#issuecomment-<id>`.
fn linked_comment_ids(body: &str) -> Vec<u64> {
    const NEEDLE: &str = "#issuecomment-";
    body.match_indices(NEEDLE)
        .filter_map(|(at, _)| {
            let digits: String = body[at + NEEDLE.len()..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            digits.parse().ok()
        })
        .collect()
}

/// The logins `body` @mentions, in order, without duplicates.
///
/// An `@` directly after a word character is an email address or a decorator,
/// not a mention. Login syntax: alphanumerics and single hyphens, at most 39.
fn mentions(body: &str) -> Vec<String> {
    let mut logins: Vec<String> = Vec::new();
    let chars: Vec<char> = body.chars().collect();
    for (at, ch) in chars.iter().enumerate() {
        if *ch != '@' {
            continue;
        }
        if at > 0 && (chars[at - 1].is_alphanumeric() || matches!(chars[at - 1], '_' | '`')) {
            continue;
        }
        let login: String = chars[at + 1..]
            .iter()
            .take_while(|c| c.is_ascii_alphanumeric() || **c == '-')
            .collect();
        let login = login.trim_end_matches('-');
        if login.is_empty() || login.len() > 39 || login.starts_with('-') {
            continue;
        }
        let login = login.to_string();
        if !logins.contains(&login) {
            logins.push(login);
        }
    }
    logins
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reply_in_a_thread_answers_the_key_its_root_opened() {
        let opened = opened_by_review_comment(10, None);
        let joined = opened_by_review_comment(11, Some(10));
        assert_eq!(opened, joined);
        assert_eq!(answered_by_review_comment(Some(10)), opened);
        assert!(answered_by_review_comment(None).is_empty());
    }

    #[test]
    fn a_conversation_comment_answers_a_comment_it_links_to() {
        let opened = opened_by_conversation_comment(42, "Rebased, CI is green.");
        let answered = answered_by_conversation_comment(
            "Re https://github.com/acme/widgets/pull/7#issuecomment-42: why?",
            "alice",
        );
        assert!(answered.iter().any(|key| opened.contains(key)));
    }

    #[test]
    fn a_conversation_comment_answers_when_its_sender_was_mentioned() {
        let opened = opened_by_conversation_comment(42, "@Alice can you confirm?");
        assert!(
            answered_by_conversation_comment("Confirmed.", "alice")
                .iter()
                .any(|key| opened.contains(key))
        );
        assert!(
            !answered_by_conversation_comment("Confirmed.", "bob")
                .iter()
                .any(|key| opened.contains(key))
        );
        assert!(
            answered_by_conversation_comment("Confirmed.", "alice[bot]")
                .iter()
                .any(|key| opened.contains(key)),
            "an App mentioned as @alice sends as alice[bot]"
        );
    }

    #[test]
    fn mentions_skip_emails_and_code_and_keep_order() {
        assert_eq!(
            mentions("@bob and @alice-x, ping bob@example.com, `@decorator`, @bob again"),
            vec!["bob".to_string(), "alice-x".to_string()]
        );
        assert!(mentions("@ alone").is_empty());
    }

    #[test]
    fn linked_comment_ids_reads_every_link() {
        assert_eq!(
            linked_comment_ids("see #issuecomment-1 and /pull/2#issuecomment-33x"),
            vec![1, 33]
        );
    }
}
