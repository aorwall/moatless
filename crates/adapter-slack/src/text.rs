//! The strings the Slack Adapter renders.
//!
//! Kept apart from the code that sends them so the wording is reviewable on its
//! own: the agent's system-prompt instructions, and the task link appended to
//! an outbound reply.

use std::fmt::Write as _;

use moatless_adapter_model::outbound::TaskProvenance;

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
Rely on the `<from>` header to tell yourself apart from other participants, \
and when in doubt stay silent rather than replying to your own prior messages.\n",
            );
        }
    }

    s.push_str(
        "\n\
### Who wrote what
Every inbound message carries a `<from>...</from>` header naming its writer — \
`alice (slack: alice.w)` when they have a moatless account, `slack: bob.k` when \
they do not, and `slack: U04XY9ZQ1MN` when Slack gave up no handle either. \
Fetched thread history is prefixed the same way, `name: message`. Use it to tell \
participants apart and to know who you are answering: several people talk in one \
thread, and only some of them are talking to you.

### Mentioning users
To address someone, write `<@login>` using their moatless login — it becomes a \
real Slack mention. A login with no linked Slack account renders as plain text \
and notifies nobody. A raw Slack id also works, `<@U01ABC2DEFG>`, which is how \
you mention someone who has no moatless account. Only write a mention when you \
mean to notify that person; naming someone in passing does not need one.

A moatless login is its own identifier: a Slack handle, an email local-part, or \
a display name from Linear, GitHub or Jira is usually *not* one, so never \
mention a name you read out of another system. `moat user list` maps display \
names and emails to logins — run it whenever you have a person's name or email \
but not their login. A mention someone else wrote in the thread is the other \
source, and a near-miss: it arrives resolved to their login when they have a \
moatless account and to their Slack handle when they do not, and the two look \
alike. Wrapping it is worth doing either way; `moat user list` is what tells \
them apart.

The same `<@login>` works inside a rich payload, in every block that renders a \
mention at all: `section` and `context` text written as `mrkdwn`, a `markdown` \
block, and a table cell. A `header`, or any `plain_text` object, cannot render \
one, so a mention there is written out as `@login` and notifies nobody. A table cell's `user` element takes a login too — \
`{\"type\": \"user\", \"user_id\": \"login\"}` — so you never have to look up a \
Slack member id yourself.

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
You receive every message in the thread, not only the ones directed at you. \
A message containing `<@{bot_id}>` or your handle references you, but that alone does not mean it's addressed to you — \
people often mention the bot in passing (\"look at what @you said\", \"ask @you later\") without actually asking you anything.\n\
- **Respond** when the message is addressed *to* you — a question, request, or instruction aimed at you\n\
- **Respond** when someone asks a question or requests help only you can provide, even without a mention\n\
- **Stay silent** when you are merely referenced or named in passing — the conversation is between other people\n\
- **Stay silent** when people are talking to each other, for acknowledgments, reactions, or off-topic chat\n\
- When in doubt, stay silent rather than interjecting\n\
- To stay silent, do nothing: send no `moat messages reply`"
        );
    }

    s
}

/// Append the provenance footer an outbound reply carries, when it carries one.
///
/// mrkdwn, not Markdown: Slack has no `[text](url)`, so the link is
/// `<url|text>`, with trailing segments joined by a middle dot.
pub(super) fn with_task_link(text: &str, provenance: Option<&TaskProvenance>) -> String {
    let mut rendered = text.to_string();
    if let Some(p) = provenance {
        let _ = write!(rendered, "\n<{}|{}>", p.url, p.label());
        for detail in p.details() {
            let _ = write!(rendered, " · {detail}");
        }
    }
    rendered
}

#[cfg(test)]
mod tests {
    use super::{reply_instructions, with_task_link};
    use moatless_adapter_model::outbound::{TaskOrigin, TaskProvenance};

    /// The instructions' raw Slack id example must pass the same check the
    /// outbound path applies, not just look id-shaped. `<@login>` is excluded:
    /// it's the placeholder for the canonical form, not an id.
    #[test]
    fn every_literal_id_in_the_instructions_is_one_the_outbound_path_accepts() {
        let s = reply_instructions(None);
        let ids: Vec<&str> = s
            .split("<@")
            .skip(1)
            .filter_map(|rest| rest.split_once('>').map(|(id, _)| id))
            .filter(|id| *id != "login")
            .collect();

        assert!(!ids.is_empty(), "the instructions demonstrate a raw id");
        for id in ids {
            assert!(
                crate::auth::is_slack_user_id(id),
                "`<@{id}>` is offered as a mention the agent can write, \
                 but the outbound path would render it as plain text"
            );
        }
    }

    /// The agent is told to mention by moatless login, so it must also be told
    /// where to get one. Without this it reaches for whatever name the last
    /// system it queried printed — a Linear `displayName`, an email local-part
    /// — and the mention silently degrades to plain text.
    #[test]
    fn the_instructions_say_where_a_login_comes_from() {
        let s = reply_instructions(Some("U0000000002"));
        let directory = s
            .find("moat user list")
            .expect("the directory command that turns a name or email into a login");
        let mentioning = s
            .find("### Mentioning users")
            .expect("Mentioning users section present");
        let threads = s
            .find("### Telling threads apart")
            .expect("Telling threads apart section present");
        assert!(
            (mentioning..threads).contains(&directory),
            "it belongs in the mentioning section, where the login is asked for"
        );
    }

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

    fn provenance(task_name: Option<&str>, origin: TaskOrigin) -> TaskProvenance {
        TaskProvenance {
            url: "https://example.test/t/1".into(),
            task_name: task_name.map(str::to_string),
            workspace_name: Some("soap-app-gitops".into()),
            origin,
        }
    }

    #[test]
    fn a_bare_provenance_renders_as_a_slack_link_on_its_own_line() {
        let bare = TaskProvenance {
            url: "https://example.test/t/1".into(),
            task_name: None,
            workspace_name: None,
            origin: TaskOrigin::Unknown,
        };
        assert_eq!(
            with_task_link("done", Some(&bare)),
            "done\n<https://example.test/t/1|View task>"
        );
        assert_eq!(
            with_task_link("done", None),
            "done",
            "no provenance, no footer, no trailing newline"
        );
    }

    /// mrkdwn, not Markdown: the link is `<url|text>` and the segments follow
    /// it on the same line.
    #[test]
    fn a_loop_fired_reply_names_the_task_workspace_and_loop() {
        let p = provenance(
            Some("prod-release — 7 apps"),
            TaskOrigin::Loop {
                name: "prod-release".into(),
            },
        );
        assert_eq!(
            with_task_link("done", Some(&p)),
            "done\n<https://example.test/t/1|prod-release — 7 apps> · soap-app-gitops · loop: prod-release"
        );
    }

    #[test]
    fn a_reply_off_a_loop_names_the_person_instead() {
        let p = provenance(
            Some("investigate flake"),
            TaskOrigin::StartedBy {
                login: "albert".into(),
            },
        );
        assert!(with_task_link("done", Some(&p)).ends_with("· started by albert"));
    }

    /// A missing segment drops itself, not the footer.
    #[test]
    fn an_unknown_workspace_drops_only_its_own_segment() {
        let p = TaskProvenance {
            url: "https://example.test/t/1".into(),
            task_name: Some("t".into()),
            workspace_name: None,
            origin: TaskOrigin::Loop { name: "l".into() },
        };
        assert_eq!(
            with_task_link("done", Some(&p)),
            "done\n<https://example.test/t/1|t> · loop: l"
        );
    }
}
