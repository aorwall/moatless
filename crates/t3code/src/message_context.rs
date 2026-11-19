//! Folds a user message's `context` records into its text, the form T3's
//! client sent before `inlineMessageContext`. The agent reads only text, and
//! the client parses these blocks back into chips (`composerContextLegacy.ts`).
//!
//! A port of the client's `serializeLegacyContextMessage`; keep the output
//! byte-identical, or messages sent before and after render differently.

use std::collections::{HashMap, HashSet};

use serde_json::Value;

const HREF_PREFIX: &str = "t3-context://v1/";
const LINK_TAIL: &str = "](t3-context://v1/";
const MAX_LINK_LABEL_CHARS: usize = 512;
const MAX_HREF_SUFFIX_CHARS: usize = 200;

/// The text a turn carries once `context` (an `OrchestrationMessageContext`)
/// is folded in. Returns `text` unchanged when there is no context.
pub(crate) fn inline_message_text(text: &str, context: Option<&Value>) -> String {
    let Some(records) = context
        .and_then(|context| context.get("records"))
        .and_then(Value::as_array)
    else {
        return text.to_owned();
    };
    let by_id: HashMap<&str, &Value> = records
        .iter()
        .filter_map(|record| Some((str_field(record, "contextId")?, record)))
        .collect();
    let mut used = HashSet::new();

    let mut text = text.to_owned();
    for link in context_links(&text).into_iter().rev() {
        let Some(record) = by_id.get(link.context_id.as_str()) else {
            continue;
        };
        used.insert(link.context_id);
        let replacement = if kind(record) == "review-comment" {
            render_review_comment(record)
        } else {
            inline_label(record)
        };
        text.replace_range(link.start..link.end, &replacement);
    }

    let of_kind =
        |wanted: &'static str| records.iter().filter(move |record| kind(record) == wanted);
    let mut blocks = vec![
        render_block(
            "terminal_context",
            of_kind("terminal").map(render_terminal_entry),
        ),
        render_block(
            "element_context",
            of_kind("element").map(render_element_entry),
        ),
    ];
    blocks.extend(of_kind("preview-annotation").map(|record| {
        format!(
            "<preview_annotation>\n{}\n</preview_annotation>",
            render_preview_body(record)
        )
    }));
    blocks.extend(
        of_kind("review-comment")
            .filter(|record| str_field(record, "contextId").is_none_or(|id| !used.contains(id)))
            .map(render_review_comment),
    );

    std::iter::once(text.trim_end().to_owned())
        .chain(blocks)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

struct ContextLink {
    context_id: String,
    start: usize,
    end: usize,
}

/// The client's `collectComposerContextReferences`: every
/// `[label](t3-context://v1/<kind>/<id>)`, optionally `!`-prefixed, in order.
fn context_links(text: &str) -> Vec<ContextLink> {
    let mut links = Vec::new();
    let mut floor = 0;
    let mut search = 0;
    while let Some(found) = text[search..].find(LINK_TAIL) {
        let close = search + found;
        search = close + 1;
        let Some(start) = label_start(text, floor, close) else {
            continue;
        };
        let href_start = close + 2;
        let Some(href_len) = text[href_start..].find(|c: char| c == ')' || c.is_whitespace())
        else {
            continue;
        };
        let href = &text[href_start..href_start + href_len];
        let suffix_chars = href.len() - HREF_PREFIX.len();
        if !text[href_start + href_len..].starts_with(')')
            || suffix_chars == 0
            || href[HREF_PREFIX.len()..].chars().count() > MAX_HREF_SUFFIX_CHARS
        {
            continue;
        }
        let end = href_start + href_len + 1;
        floor = end;
        search = end;
        if let Some(context_id) = parse_href(href) {
            links.push(ContextLink {
                context_id,
                start,
                end,
            });
        }
    }
    links
}

/// Where the match ending at `close` (the `]`) begins: the leftmost `[`
/// whose label holds no `]` or newline and fits the length cap, plus a
/// preceding `!`. Leftmost because the client's regex scans left to right.
fn label_start(text: &str, floor: usize, close: usize) -> Option<usize> {
    let mut bracket = None;
    // `label_chars` counts the chars between `c` and `close`.
    for (label_chars, (offset, c)) in text[floor..close].char_indices().rev().enumerate() {
        if c == ']' || c == '\n' {
            break;
        }
        if c == '[' {
            bracket = Some(floor + offset);
        }
        if label_chars == MAX_LINK_LABEL_CHARS {
            break;
        }
    }
    let bracket = bracket?;
    Some(if bracket > floor && text[..bracket].ends_with('!') {
        bracket - 1
    } else {
        bracket
    })
}

fn parse_href(href: &str) -> Option<String> {
    let (kind, id) = href.strip_prefix(HREF_PREFIX)?.split_once('/')?;
    let kind_ok = kind.len() <= 40
        && kind.starts_with(|c: char| c.is_ascii_lowercase())
        && kind
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    let id_ok = (1..=128).contains(&id.len())
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    (kind_ok && id_ok).then(|| id.to_owned())
}

fn kind(record: &Value) -> &str {
    str_field(record, "kind").unwrap_or_default()
}

fn str_field<'a>(record: &'a Value, field: &str) -> Option<&'a str> {
    record.get(field).and_then(Value::as_str)
}

/// A string field the client would test for truthiness: absent, null and
/// empty all read as missing.
fn present<'a>(record: &'a Value, field: &str) -> Option<&'a str> {
    str_field(record, field).filter(|value| !value.is_empty())
}

fn int_field(record: &Value, field: &str) -> Option<i64> {
    record.get(field).and_then(Value::as_i64)
}

fn inline_label(record: &Value) -> String {
    if let Some(label) = str_field(record, "terminalLabel").filter(|_| kind(record) == "terminal") {
        let slug = label
            .trim()
            .to_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join("-");
        let (start, end) = line_range(record);
        let range = if start == end {
            start.to_string()
        } else {
            format!("{start}-{end}")
        };
        return format!("@{slug}:{range}");
    }
    str_field(record, "label").unwrap_or_default().to_owned()
}

fn line_range(record: &Value) -> (i64, i64) {
    (
        int_field(record, "lineStart").unwrap_or_default(),
        int_field(record, "lineEnd").unwrap_or_default(),
    )
}

fn render_block(tag: &str, entries: impl Iterator<Item = String>) -> String {
    let body = entries
        .filter(|entry| !entry.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if body.is_empty() {
        body
    } else {
        format!("<{tag}>\n{body}\n</{tag}>")
    }
}

fn indent(value: &str) -> String {
    value
        .split('\n')
        .map(|line| {
            if line.is_empty() {
                String::new()
            } else {
                format!("  {line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_terminal_entry(record: &Value) -> String {
    let Some(label) = str_field(record, "terminalLabel") else {
        return String::new();
    };
    let (start, end) = line_range(record);
    let range = if start == end {
        format!("line {start}")
    } else {
        format!("lines {start}-{end}")
    };
    // A trailing newline would otherwise number a line past the declared range.
    let count = usize::try_from(end - start + 1).unwrap_or_default();
    let body = str_field(record, "text")
        .unwrap_or_default()
        .split('\n')
        .take(count)
        .zip(start..)
        .map(|(line, number)| format!("{number} | {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    format!("- {label} {range}:\n{}", indent(&body))
}

fn render_element_entry(record: &Value) -> String {
    if record.get("tagName").is_none() {
        return String::new();
    }
    render_element_details(record)
}

fn render_element_details(record: &Value) -> String {
    let mut lines = Vec::new();
    if let Some(url) = present(record, "pageUrl") {
        lines.push(format!("url: {url}"));
    }
    if let Some(selector) = present(record, "selector") {
        lines.push(format!("selector: {selector}"));
    }
    let source = record.get("source");
    if let Some((source, file)) =
        source.and_then(|source| Some((source, present(source, "fileName")?)))
    {
        let mut parts = vec![file.to_owned()];
        parts.extend(
            ["lineNumber", "columnNumber"]
                .into_iter()
                .filter_map(|field| int_field(source, field).map(|n| n.to_string())),
        );
        lines.push(format!("source: {}", parts.join(":")));
    }
    if let Some(html) = present(record, "htmlPreview") {
        lines.push(format!("html:\n{}", indent(html)));
    }
    if let Some(styles) = present(record, "styles") {
        lines.push(format!("styles:\n{}", indent(styles)));
    }
    let name = str_field(record, "componentName")
        .or_else(|| str_field(record, "tagName"))
        .unwrap_or_default();
    format!("- <{name}>:\n{}", indent(&lines.join("\n")))
}

fn render_preview_body(record: &Value) -> String {
    let Some(annotation) = str_field(record, "annotationId") else {
        return String::new();
    };
    let page = present(record, "pageUrl")
        .or_else(|| present(record, "pageTitle"))
        .unwrap_or_default();
    let mut lines = vec![format!("Id: {annotation}"), format!("Page: {page}")];
    if let Some(comment) = present(record, "comment") {
        lines.push(format!("Comment: {comment}"));
    }
    if let Some(targets) = present(record, "targetSummary") {
        lines.push(format!("Targets: {targets}"));
    }
    let changes: Vec<&str> = record
        .get("styleChanges")
        .and_then(Value::as_array)
        .map_or_default(|changes| changes.iter().filter_map(Value::as_str).collect());
    if !changes.is_empty() {
        lines.push("Requested visual changes:".to_owned());
        lines.extend(changes.iter().map(|change| format!("- {change}")));
    }
    let elements = record.get("elements").and_then(Value::as_array);
    if let Some(elements) = elements.filter(|elements| !elements.is_empty()) {
        lines.push("<element_context>".to_owned());
        lines.push(
            elements
                .iter()
                .map(render_element_details)
                .collect::<Vec<_>>()
                .join("\n"),
        );
        lines.push("</element_context>".to_owned());
    }
    lines.join("\n")
}

fn escape_attribute(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn render_review_comment(record: &Value) -> String {
    let Some(section_id) = str_field(record, "sectionId") else {
        return str_field(record, "label").unwrap_or_default().to_owned();
    };
    let attribute = |field: &str| escape_attribute(str_field(record, field).unwrap_or_default());
    let attributes = [
        format!("sectionId=\"{}\"", escape_attribute(section_id)),
        format!("sectionTitle=\"{}\"", attribute("sectionTitle")),
        format!("filePath=\"{}\"", attribute("filePath")),
        format!("rangeLabel=\"{}\"", attribute("rangeLabel")),
        format!(
            "startIndex=\"{}\"",
            int_field(record, "startIndex").unwrap_or_default()
        ),
        format!(
            "endIndex=\"{}\"",
            int_field(record, "endIndex").unwrap_or_default()
        ),
    ]
    .join(" ");
    let text = str_field(record, "text").unwrap_or_default();
    let body = match present(record, "diff") {
        Some(diff) => {
            // Longer than any backtick run in the diff, so the body stays fenced.
            let longest_run = diff
                .split(|c| c != '`')
                .map(str::len)
                .max()
                .unwrap_or_default();
            let fence = "`".repeat((longest_run + 1).max(3));
            let language = str_field(record, "fenceLanguage").unwrap_or("diff");
            format!("{text}\n\n{fence}{language}\n{diff}\n{fence}")
        }
        None => text.to_owned(),
    };
    format!("<review_comment {attributes}>\n{body}\n</review_comment>")
}

#[cfg(test)]
mod tests;
