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

/// Extract readable text from Slack Block Kit `blocks` array.
///
/// Walks the block structure and pulls text from:
/// - `section` / `header` → `text.text`, plus each entry of `fields[].text`
/// - `rich_text` → recursively from `elements[].elements[].text`
/// - `context` → `elements[].text`
///
/// The `section` `fields[]` array is where AWS Chatbot renders a CloudWatch
/// alarm's detail rows (reason, metric, threshold, timestamp) — without it only
/// the one-line header `text` survives and every detail is silently dropped.
///
/// Skips `actions`, `divider`, `image`, and other non-text blocks.
/// The returned string has mrkdwn references already converted via [`format_slack_text`].
pub fn extract_text_from_blocks(blocks: &[serde_json::Value]) -> String {
    let mut parts: Vec<String> = Vec::new();

    for block in blocks {
        let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
        match block_type {
            "section" | "header" => {
                if let Some(text) = block
                    .get("text")
                    .and_then(|t| t.get("text"))
                    .and_then(|t| t.as_str())
                {
                    if !text.is_empty() {
                        parts.push(text.to_string());
                    }
                }
                // A `section` may also carry a `fields[]` array of two-column
                // key/value objects (`{ "type": "mrkdwn", "text": "..." }`) —
                // AWS Chatbot puts the alarm's detail rows here. Pull each one.
                if let Some(fields) = block.get("fields").and_then(|f| f.as_array()) {
                    for field in fields {
                        if let Some(text) = field.get("text").and_then(|t| t.as_str()) {
                            if !text.is_empty() {
                                parts.push(text.to_string());
                            }
                        }
                    }
                }
            }
            "rich_text" => {
                if let Some(elements) = block.get("elements").and_then(|e| e.as_array()) {
                    for element in elements {
                        extract_rich_text_element(element, &mut parts);
                    }
                }
            }
            "context" => {
                if let Some(elements) = block.get("elements").and_then(|e| e.as_array()) {
                    for element in elements {
                        if let Some(text) = element.get("text").and_then(|t| t.as_str()) {
                            if !text.is_empty() {
                                parts.push(text.to_string());
                            }
                        }
                    }
                }
            }
            // actions, divider, image, etc. — no useful text
            _ => {}
        }
    }

    let raw = parts.join("\n");
    format_slack_text(&raw)
}

/// Recursively extract text from a `rich_text` element (e.g. `rich_text_section`,
/// `rich_text_list`, `rich_text_preformatted`, `rich_text_quote`).
fn extract_rich_text_element(element: &serde_json::Value, parts: &mut Vec<String>) {
    if let Some(inner) = element.get("elements").and_then(|e| e.as_array()) {
        let mut line_parts: Vec<String> = Vec::new();
        for child in inner {
            let child_type = child.get("type").and_then(|t| t.as_str()).unwrap_or("");
            match child_type {
                "text" | "link" | "emoji" => {
                    if let Some(text) = child.get("text").and_then(|t| t.as_str()) {
                        line_parts.push(text.to_string());
                    } else if child_type == "link" {
                        // Link without text label — use the url
                        if let Some(url) = child.get("url").and_then(|u| u.as_str()) {
                            line_parts.push(url.to_string());
                        }
                    }
                }
                "user" => {
                    if let Some(user_id) = child.get("user_id").and_then(|u| u.as_str()) {
                        line_parts.push(format!("<@{user_id}>"));
                    }
                }
                "channel" => {
                    if let Some(channel_id) = child.get("channel_id").and_then(|c| c.as_str()) {
                        line_parts.push(format!("<#{channel_id}>"));
                    }
                }
                // Nested rich_text elements (lists, quotes, etc.)
                _ if child.get("elements").is_some() => {
                    extract_rich_text_element(child, parts);
                }
                _ => {}
            }
        }
        let line = line_parts.join("");
        if !line.is_empty() {
            parts.push(line);
        }
    }
}

/// Extract readable text from Slack legacy `attachments` array.
///
/// Walks each attachment and pulls text from:
/// - `title` — attachment title (often the alert name)
/// - `text` — main body text
/// - `pretext` — text shown above the attachment
/// - `fields[]` — title/value pairs (e.g., severity: critical)
/// - `fallback` — last resort plain-text summary
///
/// Used by Prometheus Alertmanager, Grafana, and other webhook-based integrations
/// that send structured content via attachments rather than Block Kit blocks.
pub fn extract_text_from_attachments(attachments: &[serde_json::Value]) -> String {
    let mut parts: Vec<String> = Vec::new();

    for attachment in attachments {
        // pretext appears above the attachment
        if let Some(pretext) = attachment.get("pretext").and_then(|v| v.as_str()) {
            if !pretext.is_empty() {
                parts.push(pretext.to_string());
            }
        }

        // title (often the alert name)
        if let Some(title) = attachment.get("title").and_then(|v| v.as_str()) {
            if !title.is_empty() {
                parts.push(title.to_string());
            }
        }

        // main body text
        if let Some(text) = attachment.get("text").and_then(|v| v.as_str()) {
            if !text.is_empty() {
                parts.push(text.to_string());
            }
        }

        // fields (key-value pairs like severity: critical)
        if let Some(fields) = attachment.get("fields").and_then(|f| f.as_array()) {
            for field in fields {
                let title = field.get("title").and_then(|t| t.as_str()).unwrap_or("");
                let value = field.get("value").and_then(|v| v.as_str()).unwrap_or("");
                if !title.is_empty() || !value.is_empty() {
                    parts.push(format!("{title}: {value}"));
                }
            }
        }

        // fallback — only if nothing else was found for this attachment
        if parts.is_empty() {
            if let Some(fallback) = attachment.get("fallback").and_then(|v| v.as_str()) {
                if !fallback.is_empty() {
                    parts.push(fallback.to_string());
                }
            }
        }
    }

    let raw = parts.join("\n");
    format_slack_text(&raw)
}

/// Convert Slack mrkdwn markup to readable markdown/plain text.
pub fn format_slack_text(text: &str) -> String {
    let mut result = text.to_string();

    // Order matters: subteam before special mentions, URL with label before bare URL.

    // <!subteam^S123|@team-name> → @team-name
    result = SUBTEAM_MENTION
        .replace_all(&result, |caps: &regex::Captures| match caps.get(1) {
            Some(name) => format!("@{}", name.as_str()),
            None => "@team".to_string(),
        })
        .into_owned();

    // <!here> → @here, <!channel> → @channel, <!everyone> → @everyone
    result = SPECIAL_MENTION
        .replace_all(&result, |caps: &regex::Captures| format!("@{}", &caps[1]))
        .into_owned();

    // <#C123|channel-name> → #channel-name
    result = CHANNEL_LINK
        .replace_all(&result, |caps: &regex::Captures| format!("#{}", &caps[2]))
        .into_owned();

    // <@U123|display_name> → @display_name, <@U123> → @U123
    result = USER_MENTION
        .replace_all(&result, |caps: &regex::Captures| match caps.get(2) {
            Some(name) => format!("@{}", name.as_str()),
            None => format!("@{}", &caps[1]),
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
        assert_eq!(format_slack_text("see <#C456DEF|general>"), "see #general");
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

    // --- extract_text_from_blocks tests ---

    #[test]
    fn blocks_section() {
        let blocks = serde_json::json!([
            {
                "type": "section",
                "text": { "type": "mrkdwn", "text": "Hello world" }
            }
        ]);
        assert_eq!(
            extract_text_from_blocks(blocks.as_array().unwrap()),
            "Hello world"
        );
    }

    #[test]
    fn blocks_header() {
        let blocks = serde_json::json!([
            {
                "type": "header",
                "text": { "type": "plain_text", "text": "Alert Title" }
            }
        ]);
        assert_eq!(
            extract_text_from_blocks(blocks.as_array().unwrap()),
            "Alert Title"
        );
    }

    #[test]
    fn blocks_context() {
        let blocks = serde_json::json!([
            {
                "type": "context",
                "elements": [
                    { "type": "mrkdwn", "text": "env: production" },
                    { "type": "mrkdwn", "text": "region: us-east-1" }
                ]
            }
        ]);
        assert_eq!(
            extract_text_from_blocks(blocks.as_array().unwrap()),
            "env: production\nregion: us-east-1"
        );
    }

    #[test]
    fn blocks_rich_text() {
        let blocks = serde_json::json!([
            {
                "type": "rich_text",
                "elements": [
                    {
                        "type": "rich_text_section",
                        "elements": [
                            { "type": "text", "text": "Check the " },
                            { "type": "link", "url": "https://example.com", "text": "dashboard" }
                        ]
                    }
                ]
            }
        ]);
        assert_eq!(
            extract_text_from_blocks(blocks.as_array().unwrap()),
            "Check the dashboard"
        );
    }

    #[test]
    fn blocks_rich_text_user_mention() {
        let blocks = serde_json::json!([
            {
                "type": "rich_text",
                "elements": [
                    {
                        "type": "rich_text_section",
                        "elements": [
                            { "type": "text", "text": "Hey " },
                            { "type": "user", "user_id": "U123ABC" },
                            { "type": "text", "text": " check this" }
                        ]
                    }
                ]
            }
        ]);
        // format_slack_text converts <@U123ABC> → @U123ABC
        assert_eq!(
            extract_text_from_blocks(blocks.as_array().unwrap()),
            "Hey @U123ABC check this"
        );
    }

    #[test]
    fn blocks_mixed_grafana_alert() {
        // Realistic Grafana alert payload
        let blocks = serde_json::json!([
            {
                "type": "header",
                "text": { "type": "plain_text", "text": "[FIRING:1] High CPU Usage" }
            },
            {
                "type": "section",
                "text": { "type": "mrkdwn", "text": "*Status*: Firing\n*Severity*: critical" }
            },
            { "type": "divider" },
            {
                "type": "context",
                "elements": [
                    { "type": "mrkdwn", "text": "Grafana v10.2.0" }
                ]
            }
        ]);
        let result = extract_text_from_blocks(blocks.as_array().unwrap());
        assert!(result.contains("[FIRING:1] High CPU Usage"));
        assert!(result.contains("*Status*: Firing"));
        assert!(result.contains("Grafana v10.2.0"));
        // divider should not contribute text
        assert!(!result.contains("divider"));
    }

    #[test]
    fn blocks_section_fields_are_extracted() {
        // AWS Chatbot renders a CloudWatch alarm as a header `text` line plus
        // the detail rows in a `section`'s `fields[]` array. Every field must
        // survive, not just the header, and mrkdwn references convert.
        let blocks = serde_json::json!([
            {
                "type": "section",
                "text": { "type": "mrkdwn", "text": ":rotating_light: *CloudWatch Alarm* | soap-infra-prod-vpc-egress-anomaly" },
                "fields": [
                    { "type": "mrkdwn", "text": "*Reason:*\nThreshold Crossed: egress bytes above band" },
                    { "type": "mrkdwn", "text": "*Region:*\neu-north-1" },
                    { "type": "mrkdwn", "text": "*Account:*\n902916048039" }
                ]
            }
        ]);
        let result = extract_text_from_blocks(blocks.as_array().unwrap());
        assert!(result.contains("*CloudWatch Alarm* | soap-infra-prod-vpc-egress-anomaly"));
        assert!(result.contains("Threshold Crossed: egress bytes above band"));
        assert!(result.contains("eu-north-1"));
        assert!(result.contains("902916048039"));
    }

    #[test]
    fn blocks_section_fields_only_without_text() {
        // A `section` may carry `fields[]` with no top-level `text` at all.
        let blocks = serde_json::json!([
            {
                "type": "section",
                "fields": [
                    { "type": "mrkdwn", "text": "*State:*\nALARM" },
                    { "type": "mrkdwn", "text": "*Metric:*\nEgressBytes" }
                ]
            }
        ]);
        assert_eq!(
            extract_text_from_blocks(blocks.as_array().unwrap()),
            "*State:*\nALARM\n*Metric:*\nEgressBytes"
        );
    }

    #[test]
    fn blocks_empty_array() {
        let blocks = serde_json::json!([]);
        assert_eq!(extract_text_from_blocks(blocks.as_array().unwrap()), "");
    }

    #[test]
    fn blocks_skips_non_text_types() {
        let blocks = serde_json::json!([
            { "type": "divider" },
            { "type": "actions", "elements": [] },
            { "type": "image", "image_url": "https://example.com/img.png", "alt_text": "img" }
        ]);
        assert_eq!(extract_text_from_blocks(blocks.as_array().unwrap()), "");
    }

    #[test]
    fn blocks_link_without_text_falls_back_to_url() {
        let blocks = serde_json::json!([
            {
                "type": "rich_text",
                "elements": [
                    {
                        "type": "rich_text_section",
                        "elements": [
                            { "type": "link", "url": "https://example.com/dashboard" }
                        ]
                    }
                ]
            }
        ]);
        assert_eq!(
            extract_text_from_blocks(blocks.as_array().unwrap()),
            "https://example.com/dashboard"
        );
    }

    #[test]
    fn blocks_mrkdwn_in_section_converted() {
        let blocks = serde_json::json!([
            {
                "type": "section",
                "text": { "type": "mrkdwn", "text": "Ping <@U999|alice> in <#C456|ops>" }
            }
        ]);
        assert_eq!(
            extract_text_from_blocks(blocks.as_array().unwrap()),
            "Ping @alice in #ops"
        );
    }

    // --- extract_text_from_attachments tests ---

    #[test]
    fn attachments_prometheus_alert() {
        let attachments = serde_json::json!([
            {
                "color": "danger",
                "title": "[FIRING:1] HighMemoryUsage",
                "text": "Memory usage is above 90% on node-1",
                "fields": [
                    { "title": "severity", "value": "critical", "short": true },
                    { "title": "instance", "value": "10.0.0.1:9090", "short": true }
                ],
                "fallback": "HighMemoryUsage firing"
            }
        ]);
        let result = extract_text_from_attachments(attachments.as_array().unwrap());
        assert!(result.contains("[FIRING:1] HighMemoryUsage"));
        assert!(result.contains("Memory usage is above 90% on node-1"));
        assert!(result.contains("severity: critical"));
        assert!(result.contains("instance: 10.0.0.1:9090"));
    }

    #[test]
    fn attachments_with_pretext() {
        let attachments = serde_json::json!([
            {
                "pretext": "New alert from Grafana",
                "title": "CPU Spike",
                "text": "CPU at 98%"
            }
        ]);
        let result = extract_text_from_attachments(attachments.as_array().unwrap());
        assert!(result.starts_with("New alert from Grafana"));
        assert!(result.contains("CPU Spike"));
        assert!(result.contains("CPU at 98%"));
    }

    #[test]
    fn attachments_fallback_only() {
        let attachments = serde_json::json!([
            {
                "fallback": "Alert: DiskFull on prod-db-1"
            }
        ]);
        assert_eq!(
            extract_text_from_attachments(attachments.as_array().unwrap()),
            "Alert: DiskFull on prod-db-1"
        );
    }

    #[test]
    fn attachments_empty() {
        let attachments = serde_json::json!([]);
        assert_eq!(
            extract_text_from_attachments(attachments.as_array().unwrap()),
            ""
        );
    }

    #[test]
    fn attachments_mrkdwn_converted() {
        let attachments = serde_json::json!([
            {
                "text": "Alert for <@U123|alice> in <#C456|ops>"
            }
        ]);
        assert_eq!(
            extract_text_from_attachments(attachments.as_array().unwrap()),
            "Alert for @alice in #ops"
        );
    }

    #[test]
    fn attachments_multiple() {
        let attachments = serde_json::json!([
            { "title": "Alert 1", "text": "First alert" },
            { "title": "Alert 2", "text": "Second alert" }
        ]);
        let result = extract_text_from_attachments(attachments.as_array().unwrap());
        assert!(result.contains("Alert 1"));
        assert!(result.contains("First alert"));
        assert!(result.contains("Alert 2"));
        assert!(result.contains("Second alert"));
    }
}
