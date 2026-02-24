//! Slack mrkdwn → readable markdown converter.
//!
//! Converts Slack-specific markup elements to plain text or standard markdown:
//! - `<@U123>` → `@user`
//! - `<#C123|channel-name>` → `#channel-name`
//! - `<https://url|display text>` → `[display text](url)`
//! - `<https://url>` → `url`
//! - `<!here>` → `@here`, `<!channel>` → `@channel`
//! - `<!subteam^S123|@team>` → `@team`

use regex::Regex;
use std::sync::LazyLock;

// Pre-compiled regexes for Slack mrkdwn patterns (constant patterns, cannot fail).
#[allow(clippy::expect_used)]
static USER_MENTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<@([UW][A-Z0-9]+)(?:\|([^>]+))?>").expect("valid regex"));
#[allow(clippy::expect_used)]
static CHANNEL_LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<#([A-Z0-9]+)\|([^>]+)>").expect("valid regex"));
#[allow(clippy::expect_used)]
static URL_WITH_LABEL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<(https?://[^|>]+)\|([^>]+)>").expect("valid regex"));
#[allow(clippy::expect_used)]
static URL_BARE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<(https?://[^>]+)>").expect("valid regex"));
#[allow(clippy::expect_used)]
static SPECIAL_MENTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<!([a-z_]+)>").expect("valid regex"));
#[allow(clippy::expect_used)]
static SUBTEAM_MENTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<!subteam\^[A-Z0-9]+(?:\|@?([^>]+))?>").expect("valid regex"));

/// Convert Slack mrkdwn markup to readable markdown/plain text.
pub fn format_slack_text(text: &str) -> String {
    let mut result = text.to_string();

    // Order matters: subteam before special mentions, URL with label before bare URL.

    // <!subteam^S123|@team-name> → @team-name
    result = SUBTEAM_MENTION
        .replace_all(&result, |caps: &regex::Captures| {
            match caps.get(1) {
                Some(name) => format!("@{}", name.as_str()),
                None => "@team".to_string(),
            }
        })
        .into_owned();

    // <!here> → @here, <!channel> → @channel, <!everyone> → @everyone
    result = SPECIAL_MENTION
        .replace_all(&result, |caps: &regex::Captures| {
            format!("@{}", &caps[1])
        })
        .into_owned();

    // <#C123|channel-name> → #channel-name
    result = CHANNEL_LINK
        .replace_all(&result, |caps: &regex::Captures| {
            format!("#{}", &caps[2])
        })
        .into_owned();

    // <@U123|display_name> → @display_name, <@U123> → @U123
    result = USER_MENTION
        .replace_all(&result, |caps: &regex::Captures| {
            match caps.get(2) {
                Some(name) => format!("@{}", name.as_str()),
                None => format!("@{}", &caps[1]),
            }
        })
        .into_owned();

    // <https://url|display text> → [display text](url)
    result = URL_WITH_LABEL
        .replace_all(&result, |caps: &regex::Captures| {
            format!("[{}]({})", &caps[2], &caps[1])
        })
        .into_owned();

    // <https://url> → url
    result = URL_BARE
        .replace_all(&result, |caps: &regex::Captures| caps[1].to_string())
        .into_owned();

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_mention_bare() {
        assert_eq!(format_slack_text("<@U123ABC>"), "@U123ABC");
    }

    #[test]
    fn user_mention_with_name() {
        assert_eq!(format_slack_text("<@U123ABC|alice>"), "@alice");
    }

    #[test]
    fn channel_link() {
        assert_eq!(
            format_slack_text("see <#C456DEF|general>"),
            "see #general"
        );
    }

    #[test]
    fn url_with_label() {
        assert_eq!(
            format_slack_text("check <https://example.com|this link>"),
            "check [this link](https://example.com)"
        );
    }

    #[test]
    fn url_bare() {
        assert_eq!(
            format_slack_text("visit <https://example.com/path>"),
            "visit https://example.com/path"
        );
    }

    #[test]
    fn special_mentions() {
        assert_eq!(format_slack_text("<!here>"), "@here");
        assert_eq!(format_slack_text("<!channel>"), "@channel");
        assert_eq!(format_slack_text("<!everyone>"), "@everyone");
    }

    #[test]
    fn subteam_mention() {
        assert_eq!(
            format_slack_text("<!subteam^S123ABC|@backend-team>"),
            "@backend-team"
        );
    }

    #[test]
    fn subteam_mention_no_label() {
        assert_eq!(format_slack_text("<!subteam^S123ABC>"), "@team");
    }

    #[test]
    fn mixed_content() {
        let input = "<@U111|bob> please check <#C222|dev-ops> and <https://docs.rs|the docs>";
        let expected = "@bob please check #dev-ops and [the docs](https://docs.rs)";
        assert_eq!(format_slack_text(input), expected);
    }

    #[test]
    fn plain_text_unchanged() {
        let text = "just a normal message with no markup";
        assert_eq!(format_slack_text(text), text);
    }
}
