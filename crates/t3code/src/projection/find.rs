//! Find in thread: which of a conversation's messages a query occurs in, and
//! how often, in the shape `orchestration.searchThread` answers with.
//!
//! The client highlights the occurrences in the rendered DOM and steps through
//! them by the index this counts, so a message's count is taken over its
//! rendered text rather than its Markdown source: a link's target, a fence's
//! backticks and emphasis markers are never on screen.

use moatless_tasks::conversation::transcript::TranscriptEntry;
use moatless_types::message::{MessageType, TOOL_RESPONSE_USER_TYPE};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use serde_json::{Value, json};

use super::turn::turn_id;

/// The most entries the contract's `navigation` carries.
const NAVIGATION_MAX: usize = 17;

/// The contract's `OrchestrationV2SearchThreadInput.start`: an occurrence the
/// client is already on.
#[derive(Debug, Clone)]
pub(crate) struct FindStart {
    pub entry_id: String,
    pub occurrence: u64,
}

/// The rendered text of a message the timeline shows as a user or assistant
/// row, and `None` for every row it shows otherwise or not at all.
///
/// Mirrors `message_bubble` and `notification`: a tool response, a Sandbox
/// wake and a child's outcome notice are no message row, and a queued Message
/// is the queue's row until a turn takes it.
pub(crate) fn searchable_segments(entry: &TranscriptEntry) -> Option<Vec<String>> {
    entry.turn_number?;
    let text = entry
        .message_text
        .as_deref()
        .filter(|text| !text.is_empty())?;
    match entry.message_type {
        MessageType::User
            if entry.user_type != TOOL_RESPONSE_USER_TYPE
                && !entry.sandbox_wake
                && entry.child_outcome.is_none() =>
        {
            Some(rendered_segments(text, true))
        }
        MessageType::Assistant => Some(rendered_segments(text, false)),
        _ => None,
    }
}

/// The blocks of text `markdown` renders as, each searched on its own so a
/// query never matches across a paragraph break.
///
/// A user message renders every line break as one (`remark-breaks`), an
/// assistant message only paragraph breaks.
fn rendered_segments(markdown: &str, user: bool) -> Vec<String> {
    let mut segments = Vec::new();
    let mut text = String::new();
    let mut in_code_block = false;
    let flush = |text: &mut String, segments: &mut Vec<String>| {
        if !text.trim().is_empty() {
            segments.push(std::mem::take(text));
        }
        text.clear();
    };
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    for event in Parser::new_ext(markdown, options) {
        match event {
            Event::Start(Tag::CodeBlock(_)) => {
                flush(&mut text, &mut segments);
                in_code_block = true;
            }
            Event::End(TagEnd::CodeBlock) => {
                flush(&mut text, &mut segments);
                in_code_block = false;
            }
            Event::Start(tag) if is_block(&tag) => flush(&mut text, &mut segments),
            Event::End(tag) if is_block_end(tag) => flush(&mut text, &mut segments),
            Event::Text(value) if in_code_block => text.push_str(&value),
            Event::Text(value) | Event::Code(value) => {
                text.push_str(&value.replace("\r\n", " ").replace('\n', " "));
            }
            Event::InlineHtml(value) | Event::Html(value) if user => text.push_str(&value),
            Event::SoftBreak if user => flush(&mut text, &mut segments),
            Event::SoftBreak => text.push(' '),
            Event::HardBreak | Event::Rule => flush(&mut text, &mut segments),
            _ => {}
        }
    }
    flush(&mut text, &mut segments);
    segments
}

fn is_block(tag: &Tag<'_>) -> bool {
    matches!(
        tag,
        Tag::Paragraph
            | Tag::Heading { .. }
            | Tag::BlockQuote(_)
            | Tag::HtmlBlock
            | Tag::List(_)
            | Tag::Item
            | Tag::Table(_)
            | Tag::TableHead
            | Tag::TableRow
            | Tag::TableCell
            | Tag::FootnoteDefinition(_)
    )
}

fn is_block_end(tag: TagEnd) -> bool {
    matches!(
        tag,
        TagEnd::Paragraph
            | TagEnd::Heading(_)
            | TagEnd::BlockQuote(_)
            | TagEnd::HtmlBlock
            | TagEnd::List(_)
            | TagEnd::Item
            | TagEnd::Table
            | TagEnd::TableHead
            | TagEnd::TableRow
            | TagEnd::TableCell
            | TagEnd::FootnoteDefinition
    )
}

/// The non-overlapping, case-insensitive occurrences of `query` in `text`, as
/// the client's `countThreadSearchOccurrences` counts them.
pub(crate) fn count_occurrences(text: &str, query: &str) -> u64 {
    let query = fold_case(query);
    if text.is_empty() || query.is_empty() {
        return 0;
    }
    let text = fold_case(text);
    let mut count = 0;
    let mut cursor = 0;
    while let Some(offset) = text[cursor..].find(&query) {
        count += 1;
        cursor += offset + query.len();
    }
    count
}

/// Lower case, a final sigma as a medial one, and a curly apostrophe as a
/// straight one: what an iOS keyboard types must find what an agent wrote.
///
/// Per character, keeping a character whose lower case is a different length
/// in UTF-16, as the client does, so the client's offsets into the original
/// text stay valid.
fn fold_case(text: &str) -> String {
    let mut folded = String::with_capacity(text.len());
    for character in text.chars() {
        let lowered: String = character.to_lowercase().collect();
        let units = |text: &str| text.encode_utf16().count();
        if units(&lowered) == character.len_utf16() {
            for lowered in lowered.chars() {
                folded.push(match lowered {
                    'ς' => 'σ',
                    '\u{2018}' | '\u{2019}' => '\'',
                    other => other,
                });
            }
        } else {
            folded.push(character);
        }
    }
    folded
}

/// One message with at least one occurrence.
#[derive(Debug, Clone)]
struct Document {
    entry_id: String,
    run_id: Option<String>,
    count: u64,
    start_index: u64,
}

/// The documents of one search, fed oldest message first.
///
/// Holds only the messages that match, so a long conversation is searched in
/// the memory its matches take rather than its text.
#[derive(Debug)]
pub(crate) struct ThreadFind {
    task_id: String,
    query: String,
    index: Option<u64>,
    offset: i64,
    start: Option<FindStart>,
    documents: Vec<Document>,
    total: u64,
    /// The occurrences before the `start` message and in it, once it has
    /// been fed.
    anchor: Option<(u64, u64)>,
    early_sent: bool,
}

impl ThreadFind {
    pub(crate) fn new(
        task_id: &str,
        query: &str,
        index: Option<u64>,
        offset: Option<i64>,
        start: Option<FindStart>,
    ) -> Self {
        Self {
            task_id: task_id.to_string(),
            query: query.to_string(),
            index,
            offset: offset.unwrap_or(0),
            start,
            documents: Vec::new(),
            total: 0,
            anchor: None,
            early_sent: false,
        }
    }

    /// Counts one message, and answers with the progressive stream's first
    /// frame when this is the first occurrence at or after `start`.
    ///
    /// A first occurrence that wraps round to before `start` is known only
    /// once every message is fed, so no early frame is sent for it.
    pub(crate) fn feed(
        &mut self,
        entry: &TranscriptEntry,
        snapshot_sequence: u64,
    ) -> Option<Value> {
        let segments = searchable_segments(entry)?;
        let count: u64 = segments
            .iter()
            .map(|segment| count_occurrences(segment, &self.query))
            .sum();
        let is_anchor = self
            .start
            .as_ref()
            .is_some_and(|start| start.entry_id == entry.uuid);
        if is_anchor {
            self.anchor = Some((self.total, count));
        }
        let run_id = entry.turn_number.map(|turn| turn_id(&self.task_id, turn));
        if count > 0 {
            self.documents.push(Document {
                entry_id: entry.uuid.clone(),
                run_id: run_id.clone(),
                count,
                start_index: self.total,
            });
            self.total += count;
        }
        let progressive = self.index.is_none() && self.offset == 0;
        let past_anchor = self.start.is_none() || self.anchor.is_some();
        if !progressive || self.early_sent || !past_anchor {
            return None;
        }
        let occurrence = match &self.start {
            Some(start) if is_anchor => start.occurrence,
            _ => 0,
        };
        if occurrence >= count {
            return None;
        }
        self.early_sent = true;
        Some(json!({
            "complete": false,
            "snapshotSequence": snapshot_sequence,
            "totalMatches": 1,
            "activeIndex": 0,
            "match": { "entryId": entry.uuid, "runId": run_id, "occurrence": occurrence },
        }))
    }

    /// The answer once every message has been fed: upstream's `selectMatch`
    /// and `resultForSelection` over the documents.
    pub(crate) fn finish(self, snapshot_sequence: u64) -> Value {
        let total = self.total;
        let mut requested = self.index.unwrap_or(0);
        if self.index.is_none()
            && let (Some(start), Some((before, count))) = (&self.start, self.anchor)
        {
            requested = before + start.occurrence.min(count);
            if requested >= total {
                requested = 0;
            }
        }
        let active = if self.index.is_none() && total > 0 {
            let total = i128::from(total);
            let shifted = (i128::from(requested) + i128::from(self.offset)).rem_euclid(total);
            u64::try_from(shifted).unwrap_or(0)
        } else {
            requested.min(total.saturating_sub(1))
        };
        let selected = self
            .documents
            .iter()
            .position(|doc| active >= doc.start_index && active < doc.start_index + doc.count);
        let entries: Vec<Value> = navigation(&self.documents, selected)
            .into_iter()
            .map(|doc| {
                json!({
                    "entryId": doc.entry_id,
                    "runId": doc.run_id,
                    "startIndex": doc.start_index,
                    "count": doc.count,
                })
            })
            .collect();
        let matched = selected.map_or(Value::Null, |position| {
            let doc = &self.documents[position];
            json!({
                "entryId": doc.entry_id,
                "runId": doc.run_id,
                "occurrence": active - doc.start_index,
            })
        });
        json!({
            "snapshotSequence": snapshot_sequence,
            "totalMatches": total,
            "activeIndex": active,
            "navigation": entries,
            "match": matched,
        })
    }
}

/// Every document when they fit the contract's cap, else the window of
/// [`NAVIGATION_MAX`] around the selected one, wrapping at either end.
fn navigation(documents: &[Document], selected: Option<usize>) -> Vec<&Document> {
    let len = documents.len();
    if len <= NAVIGATION_MAX {
        return documents.iter().collect();
    }
    // Upstream's `findIndex` answers -1 for no selection, and the window is
    // taken from there.
    let selected = selected.map_or(-1, |index| i64::try_from(index).unwrap_or(0));
    let len_signed = i64::try_from(len).unwrap_or(i64::MAX);
    (0..NAVIGATION_MAX)
        .map(|step| {
            let step = i64::try_from(step).unwrap_or(0);
            let index = (selected + step - 8).rem_euclid(len_signed);
            &documents[usize::try_from(index).unwrap_or(0)]
        })
        .collect()
}

#[cfg(test)]
#[path = "tests/find.rs"]
mod tests;
