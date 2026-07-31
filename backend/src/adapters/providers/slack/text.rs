//! The strings the Slack Adapter renders.
//!
//! Kept apart from the code that sends them so the wording is reviewable on its
//! own: the agent's system-prompt instructions, and the task link appended to an
//! outbound reply.

use std::fmt::Write as _;

/// Build Slack-specific reply instructions, with the bot's own Slack identity
/// stated up-front so the agent always knows who it is in the thread.
pub(super) fn reply_instructions(bot_user_id: Option<&str>) -> String {
    let mut s = String::from("## Slack-specific instructions\n\n### Your identity\n");

    match bot_user_id {
        Some(bot_id) => {
            let _ = writeln!(
                s,
                "You are posting to Slack as user `<@{bot_id}>` (Slack user ID `{bot_id}`). \
Any message from `<@{bot_id}>` in the thread is something you already said — do not treat it as a new request. \
When someone writes `<@{bot_id}>` they are addressing you directly."
            );
        }
        None => {
            s.push_str(
                "Your Slack user ID is not configured. \
Rely on sender labels (`Name (login, Slack: U...): ...`) to tell yourself apart from other participants, \
and when in doubt stay silent rather than replying to your own prior messages.\n",
            );
        }
    }

    s.push_str(
        "\n\
### Mentioning users
Inbound messages carry the sender's moatless login in a `<sent_by>...</sent_by>` \
header. To @-mention a Slack user in your reply, use their Slack user id with the \
format `<@U12345>`.

### Telling threads apart
Each inbound message also carries a `<thread>...</thread>` header identifying \
the Slack thread it belongs to. Messages sharing the same `<thread>` value are \
the same conversation; different values are separate threads. Use it to keep \
concurrent conversations distinct and to recognise a continuation of an earlier thread.

### Replying to a specific message
Every inbound message has a `<message_id>...</message_id>` header. Pass that id \
to `moat messages reply` so the reply lands in the right Slack thread \
(it threads to the right place automatically):
- `moat messages reply --message <id> --text \"Here are the results\"`

### Attaching files
Add `--file` flags to attach screenshots, logs, or generated artifacts to the reply:
- `moat messages reply --message <id> --text \"Here are the results\" --file screenshot.png`
- `moat messages reply --message <id> --text \"Logs attached\" --file output.log --file error.log`
- Multiple `--file` flags can be used to attach several files at once.
- Images will render inline in the Slack thread.",
    );

    if let Some(bot_id) = bot_user_id {
        let _ = write!(
            s,
            "\n\n\
### When to respond in thread conversations\n\
You will receive ALL messages in the thread, not just ones directed at you. \
A message containing `<@{bot_id}>` or your handle references you, but that alone does not mean it's addressed to you — \
people often mention the bot in passing (\"look at what @you said\", \"ask @you later\") without actually asking you anything.\n\
- **Respond** when the message is addressed *to* you — a question, request, or instruction aimed at you\n\
- **Respond** when someone asks a question or requests help only you can provide, even without a mention\n\
- **Stay silent** when you are merely referenced or named in passing — the conversation is between other people\n\
- **Stay silent** when people are talking to each other, for acknowledgments, reactions, or off-topic chat\n\
- When in doubt, stay silent rather than interjecting\n\
- To stay silent, simply do nothing — do NOT send a `moat reply`"
        );
    }

    s
}

/// Append the `View task` link an outbound reply carries, when it carries one.
///
/// The direct-send path deliberately does not call this — a direct send has
/// never rendered the link, and that stays true.
pub(super) fn with_task_link(text: &str, task_url: Option<&str>) -> String {
    let mut rendered = text.to_string();
    if let Some(url) = task_url {
        let _ = write!(rendered, "\n<{url}|View task>");
    }
    rendered
}

#[cfg(test)]
mod tests {
    use super::{reply_instructions, with_task_link};

    #[test]
    fn reply_instructions_include_identity_with_bot_id() {
        let s = reply_instructions(Some("U0000000002"));
        // Identity section is present and appears before the mentioning-users block.
        let identity_idx = s
            .find("### Your identity")
            .expect("Your identity section present");
        let mentioning_idx = s
            .find("### Mentioning users")
            .expect("Mentioning users section present");
        assert!(
            identity_idx < mentioning_idx,
            "identity must come before mentioning-users"
        );
        // The identity block names the bot's Slack ID explicitly.
        assert!(s.contains("<@U0000000002>"));
        assert!(s.contains("Slack user ID `U0000000002`"));
        // Response guidance still references the ID.
        assert!(s.contains("### When to respond"));
    }

    #[test]
    fn reply_instructions_fallback_when_bot_id_missing() {
        let s = reply_instructions(None);
        assert!(s.contains("### Your identity"));
        assert!(s.contains("not configured"));
        // Without a known ID the response-guidance block is skipped.
        assert!(!s.contains("### When to respond"));
    }

    #[test]
    fn a_task_url_renders_as_a_slack_link_on_its_own_line() {
        assert_eq!(
            with_task_link("done", Some("https://example.test/t/1")),
            "done\n<https://example.test/t/1|View task>"
        );
        assert_eq!(
            with_task_link("done", None),
            "done",
            "no url, no link, no trailing newline"
        );
    }
}
