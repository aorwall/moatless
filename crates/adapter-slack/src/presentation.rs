//! Rendering the mentions an agent wrote inside an outbound presentation.
//!
//! Separate from `text`'s one-string rewrite because rich text carries no
//! mrkdwn: a `<@…>` typed into a table cell reaches Slack as literal
//! characters, so the cell pings only through a `user` element this walker
//! builds. `plain_text` can render no mention at all and degrades to `@name`.

use std::collections::HashMap;

use moatless_adapter_model::mentions::{resolve_mention, split_mentions, Mention, MentionPiece};
use moatless_ports::identity::ProviderIdentityLink;
use moatless_types::adapter::{
    SlackBlock, SlackPlainText, SlackRichTextElement, SlackRichTextSection, SlackTableCell,
    SlackText, SlackTextStyle,
};

/// Every token in a presentation, resolved once.
type Resolved = HashMap<String, Mention>;

/// Rewrite every mention in `blocks` into one Slack notifies on.
///
/// Two passes: the first resolves each distinct token once, so a login in ten
/// table rows costs one directory read; the second rewrites synchronously.
pub(super) async fn render_block_mentions(
    users: &dyn ProviderIdentityLink,
    blocks: Vec<SlackBlock>,
) -> Vec<SlackBlock> {
    let mut tokens = Vec::new();
    for block in &blocks {
        collect_block(block, &mut tokens);
    }
    tokens.sort_unstable();
    tokens.dedup();

    let mut resolved = Resolved::with_capacity(tokens.len());
    for token in tokens {
        let mention = resolve_mention(
            users,
            super::delivery::ADAPTER,
            &token,
            super::auth::is_slack_user_id,
        )
        .await;
        resolved.insert(token, mention);
    }

    blocks
        .into_iter()
        .map(|block| rewrite_block(block, &resolved))
        .collect()
}

// =============================================================================
// Pass 1: what needs looking up
// =============================================================================
//
// Only the surfaces that can render a mention are collected. A `plain_text`
// string is rewritten from the token alone, so collecting it would buy a
// directory read nothing uses.

fn collect_block(block: &SlackBlock, out: &mut Vec<String>) {
    match block {
        SlackBlock::Section { text, fields } => {
            for text in text.iter().chain(fields.iter().flatten()) {
                collect_text(text, out);
            }
        }
        SlackBlock::Markdown { text } => collect_str(text, out),
        SlackBlock::Table { rows } => {
            for element in rows.iter().flatten().flat_map(cell_elements) {
                match element {
                    SlackRichTextElement::Text { text, .. } => collect_str(text, out),
                    SlackRichTextElement::User { user_id } => out.push(user_id.clone()),
                    SlackRichTextElement::Link { .. } => {}
                }
            }
        }
        SlackBlock::Context { elements } => {
            for text in elements {
                collect_text(text, out);
            }
        }
        SlackBlock::Header { .. } | SlackBlock::Divider | SlackBlock::Image { .. } => {}
    }
}

fn collect_text(text: &SlackText, out: &mut Vec<String>) {
    match text {
        SlackText::Mrkdwn { text } => collect_str(text, out),
        SlackText::PlainText { .. } => {}
    }
}

fn collect_str(text: &str, out: &mut Vec<String>) {
    for piece in split_mentions(text) {
        if let MentionPiece::Token(token) = piece {
            out.push(token.to_string());
        }
    }
}

/// The leaves of a cell, flattened across its sections.
fn cell_elements(cell: &SlackTableCell) -> impl Iterator<Item = &SlackRichTextElement> {
    let SlackTableCell::RichText { elements } = cell;
    elements.iter().flat_map(|section| {
        let SlackRichTextSection::RichTextSection { elements } = section;
        elements.iter()
    })
}

// =============================================================================
// Pass 2: the rewrite
// =============================================================================

fn rewrite_block(block: SlackBlock, resolved: &Resolved) -> SlackBlock {
    match block {
        SlackBlock::Header { text } => {
            let SlackPlainText::PlainText { text, emoji } = text;
            SlackBlock::Header {
                text: SlackPlainText::PlainText {
                    text: plain(&text),
                    emoji,
                },
            }
        }
        SlackBlock::Section { text, fields } => SlackBlock::Section {
            text: text.map(|t| rewrite_text(t, resolved)),
            fields: fields.map(|f| f.into_iter().map(|t| rewrite_text(t, resolved)).collect()),
        },
        SlackBlock::Markdown { text } => SlackBlock::Markdown {
            text: mrkdwn(&text, resolved),
        },
        SlackBlock::Table { rows } => SlackBlock::Table {
            rows: rows
                .into_iter()
                .map(|row| row.into_iter().map(|c| rewrite_cell(c, resolved)).collect())
                .collect(),
        },
        SlackBlock::Context { elements } => SlackBlock::Context {
            elements: elements
                .into_iter()
                .map(|t| rewrite_text(t, resolved))
                .collect(),
        },
        block @ (SlackBlock::Divider | SlackBlock::Image { .. }) => block,
    }
}

fn rewrite_text(text: SlackText, resolved: &Resolved) -> SlackText {
    match text {
        SlackText::Mrkdwn { text } => SlackText::Mrkdwn {
            text: mrkdwn(&text, resolved),
        },
        SlackText::PlainText { text, emoji } => SlackText::PlainText {
            text: plain(&text),
            emoji,
        },
    }
}

fn rewrite_cell(cell: SlackTableCell, resolved: &Resolved) -> SlackTableCell {
    let SlackTableCell::RichText { elements } = cell;
    SlackTableCell::RichText {
        elements: elements
            .into_iter()
            .map(|section| {
                let SlackRichTextSection::RichTextSection { elements } = section;
                SlackRichTextSection::RichTextSection {
                    elements: elements
                        .into_iter()
                        .flat_map(|e| rewrite_element(e, resolved))
                        .collect(),
                }
            })
            .collect(),
    }
}

/// A mrkdwn string, where Slack resolves its own `<@id>` syntax.
fn mrkdwn(text: &str, resolved: &Resolved) -> String {
    let mut out = String::with_capacity(text.len());
    for piece in split_mentions(text) {
        match piece {
            MentionPiece::Literal(literal) => out.push_str(literal),
            MentionPiece::Token(token) => {
                out.push_str(&lookup(token, resolved).render(|id| format!("<@{id}>")));
            }
        }
    }
    out
}

/// A string on a surface that renders no mention at all — a `plain_text`
/// object, or a link's label. The login names who was meant, so it is written
/// out rather than left as `<@…>` for a reader to decipher.
fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for piece in split_mentions(text) {
        match piece {
            MentionPiece::Literal(literal) => out.push_str(literal),
            MentionPiece::Token(token) => {
                out.push('@');
                out.push_str(token);
            }
        }
    }
    out
}

fn rewrite_element(
    element: SlackRichTextElement,
    resolved: &Resolved,
) -> Vec<SlackRichTextElement> {
    match element {
        SlackRichTextElement::Text { text, style } => split_leaf(&text, style, resolved),
        SlackRichTextElement::User { user_id } => {
            vec![leaf(lookup(&user_id, resolved), None)]
        }
        SlackRichTextElement::Link { url, text, style } => {
            vec![SlackRichTextElement::Link {
                url,
                text: text.as_deref().map(plain),
                style,
            }]
        }
    }
}

/// Split one text leaf around its mentions, into alternating text and `user`
/// elements. `style` carries onto every text run the split produces, so the
/// words around a mention keep the styling the leaf declared.
fn split_leaf(
    text: &str,
    style: Option<SlackTextStyle>,
    resolved: &Resolved,
) -> Vec<SlackRichTextElement> {
    if !text.contains("<@") {
        return vec![SlackRichTextElement::Text {
            text: text.to_string(),
            style,
        }];
    }
    split_mentions(text)
        .map(|piece| match piece {
            MentionPiece::Literal(literal) => SlackRichTextElement::Text {
                text: literal.to_string(),
                style: style.clone(),
            },
            MentionPiece::Token(token) => leaf(lookup(token, resolved), style.clone()),
        })
        .collect()
}

/// The rich-text leaf one mention becomes. A `user` element can only hold an id
/// Slack knows, so anyone unlinked degrades to text rather than a `user_id`
/// Slack would reject.
fn leaf(mention: Mention, style: Option<SlackTextStyle>) -> SlackRichTextElement {
    match mention {
        Mention::Native(user_id) => SlackRichTextElement::User { user_id },
        Mention::Plain(name) => SlackRichTextElement::Text {
            text: format!("@{name}"),
            style,
        },
    }
}

/// Pass 1 collects every token pass 2 rewrites; a miss can only mean the two
/// walks disagree, and naming the person in plain text is the safe reading.
fn lookup(token: &str, resolved: &Resolved) -> Mention {
    resolved
        .get(token)
        .cloned()
        .unwrap_or_else(|| Mention::Plain(token.to_string()))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use moatless_ports::identity::{LinkedUser, ProviderIdentityLink};
    use moatless_types::store::StoreError;
    use uuid::Uuid;

    use super::*;

    /// Links `aorwall` to `U0000000001`, and counts what it was asked.
    #[derive(Default)]
    struct Directory {
        lookups: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl ProviderIdentityLink for Directory {
        async fn user_at(&self, _: &str, _: &str) -> Result<Option<LinkedUser>, StoreError> {
            Ok(None)
        }
        async fn user_by_email(&self, _: &str) -> Result<Option<LinkedUser>, StoreError> {
            Ok(None)
        }
        async fn provider_id_for_login(
            &self,
            _: &str,
            login: &str,
        ) -> Result<Option<String>, StoreError> {
            self.lookups.fetch_add(1, Ordering::Relaxed);
            Ok((login == "aorwall").then(|| "U0000000001".to_string()))
        }
        async fn user_who_connected(&self, _: &str, _: &str) -> Result<Option<String>, StoreError> {
            Ok(None)
        }
        async fn user_with_login(&self, _: &str, _: &str) -> Result<Option<String>, StoreError> {
            Ok(None)
        }
        async fn link(&self, _: &Uuid, _: &str, _: &str) -> Result<bool, StoreError> {
            Ok(false)
        }
    }

    async fn render(blocks: Vec<SlackBlock>) -> Vec<SlackBlock> {
        render_block_mentions(&Directory::default(), blocks).await
    }

    fn md(text: &str) -> SlackText {
        SlackText::Mrkdwn {
            text: text.to_string(),
        }
    }

    fn cell(elements: Vec<SlackRichTextElement>) -> SlackTableCell {
        SlackTableCell::RichText {
            elements: vec![SlackRichTextSection::RichTextSection { elements }],
        }
    }

    fn leaves(block: &SlackBlock) -> Vec<SlackRichTextElement> {
        let SlackBlock::Table { rows } = block else {
            panic!("expected a table, got {block:?}");
        };
        rows.iter().flatten().flat_map(cell_elements).cloned().collect()
    }

    fn text_of(block: &SlackBlock) -> String {
        match block {
            SlackBlock::Markdown { text } => text.clone(),
            SlackBlock::Section {
                text: Some(SlackText::Mrkdwn { text }),
                ..
            }
            | SlackBlock::Header {
                text: SlackPlainText::PlainText { text, .. },
            } => text.clone(),
            SlackBlock::Context { elements } => match elements.first() {
                Some(SlackText::Mrkdwn { text } | SlackText::PlainText { text, .. }) => text.clone(),
                None => String::new(),
            },
            other => panic!("no single text on {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_login_becomes_a_mention_in_every_mrkdwn_surface() {
        let rendered = render(vec![
            SlackBlock::Section {
                text: Some(md("ping <@aorwall>")),
                fields: Some(vec![md("owner <@aorwall>")]),
            },
            SlackBlock::Markdown {
                text: "**owner** <@aorwall>".to_string(),
            },
            SlackBlock::Context {
                elements: vec![md("filed by <@aorwall>")],
            },
        ])
        .await;

        assert_eq!(text_of(&rendered[0]), "ping <@U0000000001>");
        assert_eq!(text_of(&rendered[1]), "**owner** <@U0000000001>");
        assert_eq!(text_of(&rendered[2]), "filed by <@U0000000001>");
        let SlackBlock::Section { fields, .. } = &rendered[0] else {
            panic!("expected a section");
        };
        assert_eq!(fields.as_deref(), Some([md("owner <@U0000000001>")].as_slice()));
    }

    #[tokio::test]
    async fn a_login_in_a_table_cell_becomes_a_user_element() {
        let rendered = render(vec![SlackBlock::Table {
            rows: vec![vec![cell(vec![SlackRichTextElement::Text {
                text: "<@aorwall>".to_string(),
                style: None,
            }])]],
        }])
        .await;

        assert_eq!(
            leaves(&rendered[0]),
            [SlackRichTextElement::User {
                user_id: "U0000000001".to_string(),
            }]
        );
    }

    #[tokio::test]
    async fn a_user_element_accepts_a_login_in_place_of_a_member_id() {
        let rendered = render(vec![SlackBlock::Table {
            rows: vec![vec![cell(vec![SlackRichTextElement::User {
                user_id: "aorwall".to_string(),
            }])]],
        }])
        .await;

        assert_eq!(
            leaves(&rendered[0]),
            [SlackRichTextElement::User {
                user_id: "U0000000001".to_string(),
            }]
        );
    }

    #[tokio::test]
    async fn a_mid_string_mention_splits_its_leaf_and_every_run_keeps_the_style() {
        let bold = Some(SlackTextStyle {
            bold: Some(true),
            ..SlackTextStyle::default()
        });
        let rendered = render(vec![SlackBlock::Table {
            rows: vec![vec![cell(vec![SlackRichTextElement::Text {
                text: "owner <@aorwall> today".to_string(),
                style: bold.clone(),
            }])]],
        }])
        .await;

        assert_eq!(
            leaves(&rendered[0]),
            [
                SlackRichTextElement::Text {
                    text: "owner ".to_string(),
                    style: bold.clone(),
                },
                SlackRichTextElement::User {
                    user_id: "U0000000001".to_string(),
                },
                SlackRichTextElement::Text {
                    text: " today".to_string(),
                    style: bold,
                },
            ]
        );
    }

    #[tokio::test]
    async fn an_unlinked_login_degrades_to_a_text_leaf() {
        let rendered = render(vec![SlackBlock::Table {
            rows: vec![vec![cell(vec![SlackRichTextElement::User {
                user_id: "martin".to_string(),
            }])]],
        }])
        .await;

        assert_eq!(
            leaves(&rendered[0]),
            [SlackRichTextElement::Text {
                text: "@martin".to_string(),
                style: None,
            }]
        );
    }

    #[tokio::test]
    async fn an_unlinked_login_pings_nobody_in_mrkdwn() {
        let rendered = render(vec![SlackBlock::Section {
            text: Some(md("ping <@martin>")),
            fields: None,
        }])
        .await;

        assert_eq!(text_of(&rendered[0]), "ping @martin");
    }

    /// A header renders no mention at all, so the login is written out rather
    /// than left as `<@…>` for a reader to decipher.
    #[tokio::test]
    async fn a_header_names_the_person_in_plain_text() {
        let rendered = render(vec![SlackBlock::Header {
            text: SlackPlainText::PlainText {
                text: "Owned by <@aorwall>".to_string(),
                emoji: None,
            },
        }])
        .await;

        assert_eq!(text_of(&rendered[0]), "Owned by @aorwall");
    }

    /// A link label is display text like any other, so it names the person
    /// rather than keeping the angle brackets a reader would have to decipher.
    #[tokio::test]
    async fn a_link_label_names_the_person_in_plain_text() {
        let rendered = render(vec![SlackBlock::Table {
            rows: vec![vec![cell(vec![SlackRichTextElement::Link {
                url: "https://x.test".to_string(),
                text: Some("ask <@aorwall>".to_string()),
                style: None,
            }])]],
        }])
        .await;

        assert_eq!(
            leaves(&rendered[0]),
            [SlackRichTextElement::Link {
                url: "https://x.test".to_string(),
                text: Some("ask @aorwall".to_string()),
                style: None,
            }]
        );
    }

    /// The escape hatch, in both dialects: an id Slack can already address is
    /// never looked up, so someone Moatless has never seen stays reachable.
    #[tokio::test]
    async fn a_raw_member_id_passes_through_without_a_lookup() {
        let directory = Directory::default();
        let rendered = render_block_mentions(
            &directory,
            vec![
                SlackBlock::Section {
                    text: Some(md("ping <@U0NEVERSEEN>")),
                    fields: None,
                },
                SlackBlock::Table {
                    rows: vec![vec![cell(vec![SlackRichTextElement::User {
                        user_id: "U0NEVERSEEN".to_string(),
                    }])]],
                },
            ],
        )
        .await;

        assert_eq!(text_of(&rendered[0]), "ping <@U0NEVERSEEN>");
        assert_eq!(
            leaves(&rendered[1]),
            [SlackRichTextElement::User {
                user_id: "U0NEVERSEEN".to_string(),
            }]
        );
        assert_eq!(directory.lookups.load(Ordering::Relaxed), 0);
    }

    /// The reason resolution is a pass over the whole presentation: an owner
    /// repeated down a table column is one directory read, not one per row.
    #[tokio::test]
    async fn one_login_across_the_presentation_costs_one_lookup() {
        let directory = Directory::default();
        let row = || {
            vec![cell(vec![SlackRichTextElement::User {
                user_id: "aorwall".to_string(),
            }])]
        };
        render_block_mentions(
            &directory,
            vec![
                SlackBlock::Section {
                    text: Some(md("ping <@aorwall> and <@aorwall>")),
                    fields: None,
                },
                SlackBlock::Table {
                    rows: vec![row(), row(), row()],
                },
            ],
        )
        .await;

        assert_eq!(directory.lookups.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn a_presentation_with_no_mention_survives_unchanged() {
        let blocks = vec![
            SlackBlock::Divider,
            SlackBlock::Markdown {
                text: "install @moatless/api-client".to_string(),
            },
            SlackBlock::Table {
                rows: vec![vec![cell(vec![
                    SlackRichTextElement::Text {
                        text: "SP-1".to_string(),
                        style: None,
                    },
                    SlackRichTextElement::Link {
                        url: "https://x.test".to_string(),
                        text: Some("docs".to_string()),
                        style: None,
                    },
                ])]],
            },
        ];

        assert_eq!(render(blocks.clone()).await, blocks);
    }
}
