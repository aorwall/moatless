//! Adapter domain types: the identity of an Adapter, the contexts it is invoked
//! with, the messages that cross its boundary in either direction, and the
//! values that cross its delivery seam.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::access_control::Scope;

crate::string_enum!(
    /// Canonical external adapter identifier.
    AdapterKind [Hash] {
        Slack => "slack",
        Telegram => "telegram",
        Github => "github",
        GithubPr => "github_pr",
        Linear => "linear",
        Webhook => "webhook",
    }
);

impl AdapterKind {
    pub fn is_replyable(self) -> bool {
        true
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "slack" => Some(Self::Slack),
            "telegram" => Some(Self::Telegram),
            "github" => Some(Self::Github),
            "github_pr" => Some(Self::GithubPr),
            "linear" => Some(Self::Linear),
            "webhook" => Some(Self::Webhook),
            _ => None,
        }
    }

    /// Parse a stored `adapter_kind`, defaulting an unrecognized value to
    /// [`Webhook`](Self::Webhook) for existing rows; to reject unknowns, use
    /// [`parse`](Self::parse).
    pub fn parse_or_webhook(value: &str) -> Self {
        Self::parse(value).unwrap_or_else(|| {
            tracing::warn!(
                adapter_kind = value,
                "unrecognized adapter_kind, defaulting to webhook"
            );
            Self::Webhook
        })
    }
}

/// The origin label a schedule firing records, in `LoopContext.adapter_kind`
/// and on the message row it writes.
///
/// Not an Adapter, so [`AdapterKind::parse`] rejects it, and the row renders
/// with no inbound reply target to fail on.
pub const SCHEDULED_ORIGIN: &str = "scheduled";

// =============================================================================
// Adapter contexts
// =============================================================================

/// Normalized external status for the subject a `TaskBinding` points at.
///
/// Produced by each adapter's own derivation (GitHub PR fields, Linear workflow
/// state) and persisted onto the binding's `external_status*` columns.
#[derive(Debug, Clone)]
pub struct ExternalStatus {
    /// Display string (GitHub: `open`/`merged`/`closed`/`draft`; Linear: the
    /// workflow state name).
    pub status: Option<String>,
    /// Normalized bucket used for color-coding / filtering.
    pub state_kind: Option<String>,
    /// Full detail blob persisted to `external_status_raw`.
    pub raw: serde_json::Value,
}

/// Inbound message produced by an Adapter-specific parser.
///
/// Serialized into `adapter_delivery.payload` and `inbound_message_buffer.message`:
/// both seams redo routing from this record after a restart, so every field is
/// owned data.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InboundMessage {
    /// Adapter type.
    pub adapter_kind: AdapterKind,
    /// Source identifier (Slack channelId, email address).
    pub source_id: String,
    /// Thread ID if this is a reply (Slack `thread_ts`, Email In-Reply-To).
    pub thread_id: Option<String>,
    /// New thread ID for the message itself (Slack message ts, Email
    /// Message-ID).
    pub message_thread_id: String,
    /// Resolved user ID (system user).
    pub user_id: String,
    /// The handle the external system knows the writer by — a GitHub login,
    /// Slack display name, or Linear actor — recorded verbatim, never resolved
    /// to a Moatless identity (`user_id` holds that).
    #[serde(default)]
    pub author: Option<String>,
    /// The writer's own id in the external system, distinct from `user_id` (the
    /// subscription's owner): always names whoever wrote the message, what the
    /// sender label resolves through.
    #[serde(default)]
    pub sender_external_id: Option<String>,
    /// Message text content.
    pub text: String,
    /// File attachment IDs (already uploaded to S3).
    pub file_ids: Vec<String>,
    /// Adapter-specific properties to store on thread.
    pub thread_properties: serde_json::Value,
    /// Adapter-specific context.
    pub adapter_context: serde_json::Value,
    /// External message ID for dedup (e.g., Slack `ts`, Telegram `message_id`).
    /// Must be unique per message — NOT the thread ID.
    pub external_event_id: Option<String>,
    /// Source task id when the event was authored by a moatless task (parsed
    /// from the `Moatless task:` footer link); routing skips delivery back to
    /// it while still fanning out to other bindings.
    pub source_task_id: Option<String>,
    /// Who this event is for. See [`Audience`].
    ///
    /// `#[serde(default)]`: a record predating this field replays as
    /// [`Audience::Everyone`], matching prior routing behavior.
    #[serde(default)]
    pub audience: Audience,
}

/// The external system's handle for whoever wrote an inbound event, or `None`
/// if it named nobody. Fills [`InboundMessage::author`].
///
/// Adapters read this off `#[serde(default)]` fields, so a missing sender
/// arrives as an empty string, not an absent one.
pub fn author_handle(handle: &str) -> Option<String> {
    let trimmed = handle.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// Who an inbound event is for, as stated by the adapter that parsed it.
///
/// Classification stays with the adapter; turning an audience into recipients
/// is the router's job, through `resolve_recipients`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Audience {
    /// Every active binding on the subject.
    #[default]
    Everyone,
    /// The task that produced the subject, falling back to every active binding
    /// when no producing task is bound — so a signal about the subject's health
    /// still reaches *some* task rather than vanishing.
    SubjectProducer,
    /// The task that authored the named parent event, plus the task that
    /// produced the subject.
    ///
    /// Falls back to every active binding when neither is bound. A spawned task
    /// hears only its own threads; the producer hears replies in all of them.
    ParentAuthor {
        /// Identifier of the event being replied to, in
        /// [`InboundMessage::external_event_id`]'s namespace. `None` starts its
        /// own thread, leaving the producer (or subject) as the only recipient.
        parent_external_event_id: Option<String>,
    },
}

// =============================================================================
// Adapter-native presentation
// =============================================================================

/// Adapter-native rich rendering of an outbound message body.
///
/// The portable reply text stays mandatory — every adapter sends it, recorded
/// as the task-timeline entry; this is the optional richer form, ignored by an
/// adapter it isn't addressed to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OutboundPresentation {
    /// Slack Block Kit. Display only: there is no interactive-element variant,
    /// because nothing routes a `block_actions` click back to a waiting task.
    Slack { blocks: Vec<SlackBlock> },
}

impl OutboundPresentation {
    /// The adapter this presentation addresses.
    pub fn kind(&self) -> AdapterKind {
        match self {
            Self::Slack { .. } => AdapterKind::Slack,
        }
    }
}

/// One Slack Block Kit block.
///
/// Typed rather than a raw `Value` so a malformed payload fails where entered,
/// with a field path, instead of reaching Slack as a bare `invalid_blocks`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SlackBlock {
    /// Large bold title. Slack accepts only `plain_text` here.
    Header {
        text: SlackPlainText,
    },
    /// Body text, optionally with a two-column `fields` array.
    Section {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<SlackText>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fields: Option<Vec<SlackText>>,
    },
    /// Standard Markdown that Slack translates, headings and tables included.
    Markdown {
        text: String,
    },
    /// Tabular data. Cells are rich text, so they can carry live links.
    Table {
        rows: Vec<Vec<SlackTableCell>>,
    },
    /// Small muted line, typically a footer.
    Context {
        elements: Vec<SlackText>,
    },
    Divider,
    Image {
        image_url: String,
        alt_text: String,
    },
}

/// A Slack composition text object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SlackText {
    /// Slack mrkdwn: `*bold*`, `_italic_`, `<url|text>`.
    Mrkdwn { text: String },
    PlainText {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        emoji: Option<bool>,
    },
}

/// A `plain_text` object. Single-variant so the `type` tag is fixed: a header
/// cannot be handed the mrkdwn that Slack would reject there.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SlackPlainText {
    PlainText {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        emoji: Option<bool>,
    },
}

/// One `table` cell.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SlackTableCell {
    RichText { elements: Vec<SlackRichTextSection> },
}

/// A rich-text section inside a table cell.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SlackRichTextSection {
    RichTextSection { elements: Vec<SlackRichTextElement> },
}

/// A leaf inside a rich-text section.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SlackRichTextElement {
    Text {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        style: Option<SlackTextStyle>,
    },
    Link {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        style: Option<SlackTextStyle>,
    },
}

/// Inline styling for a rich-text leaf.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SlackTextStyle {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bold: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub italic: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strike: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<bool>,
}

// =============================================================================
// Adapter delivery
// =============================================================================
//
// A destination names where a reply lands; a payload carries everything a
// worker needs to redo inbound routing after the admitting process is gone.

/// Everything `route_inbound_message` needs, outliving the process that
/// admitted it.
///
/// Connection and Loop are referenced by id, never embedded; fields may only be
/// *added*, with `#[serde(default)]`, or a replayed payload goes terminal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundDeliveryPayload {
    pub connection_id: String,
    pub loop_id: String,
    pub subject_kind: String,
    pub subject_key: String,
    pub destination_config: serde_json::Value,
    /// The sender's own event name, carried only so the webhook outcome metric
    /// keeps the labels it had while routing ran on the request path. Empty for
    /// senders that never labelled it.
    pub event_type: String,
    pub message: InboundMessage,
}

// =============================================================================
// Adapter settings
// =============================================================================

/// Linear adapter settings.
///
/// Implements a custom `Debug` to redact secrets.
#[derive(Clone)]
pub struct LinearConfig {
    /// Global webhook secret for verifying Linear webhook signatures.
    pub webhook_secret: Option<String>,
    /// OAuth `client_id` from the Linear OAuth2 app registration.
    pub oauth_client_id: Option<String>,
    /// OAuth `client_secret` from the Linear OAuth2 app registration.
    pub oauth_client_secret: Option<String>,
    /// OAuth `redirect_uri` registered on the Linear OAuth2 app.
    pub oauth_callback_url: Option<String>,
    /// API host every Linear client talks to, GraphQL and OAuth alike (`None` =
    /// Linear itself; env `LINEAR_API_BASE_URL`).
    ///
    /// The Adapter's injection seam: `LinearAgentApi` is built deep inside
    /// paths holding only an `AppState`, so redirecting it must travel there.
    pub api_base_url: Option<String>,
}

#[allow(clippy::missing_fields_in_debug)]
impl std::fmt::Debug for LinearConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LinearConfig")
            .field(
                "webhook_secret",
                &self.webhook_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .field("oauth_client_id", &self.oauth_client_id)
            .field(
                "oauth_client_secret",
                &self.oauth_client_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .field("oauth_callback_url", &self.oauth_callback_url)
            .field("api_base_url", &self.api_base_url)
            .finish()
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "the assertions are the wire contract")]
mod tests {
    use super::*;

    /// The three shapes a webhook field arrives in. Whitespace and the empty
    /// string are the same answer as an absent field: the origin chip names who
    /// wrote it or names nobody, and never shows a blank.
    #[test]
    fn a_handle_that_names_nobody_is_nobody() {
        assert_eq!(author_handle("alice").as_deref(), Some("alice"));
        assert_eq!(author_handle("  alice  ").as_deref(), Some("alice"));
        assert_eq!(author_handle(""), None);
        assert_eq!(author_handle("   "), None);
    }

    /// An unrecognized `adapter_kind` decodes to `Webhook`, the lenient default
    /// the row readers depend on; a known value round-trips.
    #[test]
    fn an_unknown_adapter_kind_decodes_to_webhook() {
        assert_eq!(AdapterKind::parse_or_webhook("linear"), AdapterKind::Linear);
        assert_eq!(AdapterKind::parse_or_webhook("slakc"), AdapterKind::Webhook);
    }

    #[test]
    fn linear_config_debug_redacts_secrets() {
        let linear = LinearConfig {
            webhook_secret: Some("real-webhook-secret".into()),
            oauth_client_id: Some("client-123".into()),
            oauth_client_secret: Some("real-oauth-secret".into()),
            oauth_callback_url: Some("https://app/callback".into()),
            api_base_url: None,
        };
        let debug = format!("{linear:?}");
        assert!(!debug.contains("real-webhook-secret"));
        assert!(!debug.contains("real-oauth-secret"));
        assert!(debug.contains("[REDACTED]"));
        assert!(debug.contains("client-123")); // oauth_client_id is not a secret
    }

    /// Slack rejects a payload whose shape is off by one key, so the wire form
    /// of every block variant is pinned here rather than discovered in staging.
    #[test]
    fn every_block_variant_serialises_to_the_shape_slack_documents() {
        let cases: Vec<(SlackBlock, serde_json::Value)> = vec![
            (
                SlackBlock::Header {
                    text: SlackPlainText::PlainText {
                        text: "Prod release".into(),
                        emoji: Some(true),
                    },
                },
                serde_json::json!({
                    "type": "header",
                    "text": { "type": "plain_text", "text": "Prod release", "emoji": true }
                }),
            ),
            (
                SlackBlock::Section {
                    text: Some(SlackText::Mrkdwn {
                        text: "*7 apps*".into(),
                    }),
                    fields: None,
                },
                serde_json::json!({
                    "type": "section",
                    "text": { "type": "mrkdwn", "text": "*7 apps*" }
                }),
            ),
            (
                SlackBlock::Markdown {
                    text: "### Heading".into(),
                },
                serde_json::json!({ "type": "markdown", "text": "### Heading" }),
            ),
            (
                SlackBlock::Divider,
                serde_json::json!({ "type": "divider" }),
            ),
            (
                SlackBlock::Context {
                    elements: vec![SlackText::Mrkdwn {
                        text: "93 to promote".into(),
                    }],
                },
                serde_json::json!({
                    "type": "context",
                    "elements": [{ "type": "mrkdwn", "text": "93 to promote" }]
                }),
            ),
        ];

        for (block, expected) in cases {
            assert_eq!(
                serde_json::to_value(&block).expect("a block serialises"),
                expected
            );
            // and the payload an agent hands us round-trips back to the same block
            let parsed: SlackBlock = serde_json::from_value(expected).expect("it parses back");
            assert_eq!(parsed, block);
        }
    }

    /// A table cell is the one place a link survives inside tabular data, which
    /// is the whole reason the release matrix uses a table block.
    #[test]
    fn a_table_cell_carries_a_live_link() {
        let block = SlackBlock::Table {
            rows: vec![vec![SlackTableCell::RichText {
                elements: vec![SlackRichTextSection::RichTextSection {
                    elements: vec![SlackRichTextElement::Link {
                        url: "https://github.com/soap/soap-backend/compare/a...b".into(),
                        text: Some("1.0.1204 (d0eec4f3)".into()),
                        style: Some(SlackTextStyle {
                            bold: Some(true),
                            ..Default::default()
                        }),
                    }],
                }],
            }]],
        };

        assert_eq!(
            serde_json::to_value(&block).expect("a table block serialises"),
            serde_json::json!({
                "type": "table",
                "rows": [[{
                    "type": "rich_text",
                    "elements": [{
                        "type": "rich_text_section",
                        "elements": [{
                            "type": "link",
                            "url": "https://github.com/soap/soap-backend/compare/a...b",
                            "text": "1.0.1204 (d0eec4f3)",
                            "style": { "bold": true }
                        }]
                    }]
                }]]
            })
        );
    }

    #[test]
    fn a_presentation_names_the_adapter_it_addresses() {
        let pres = OutboundPresentation::Slack {
            blocks: vec![SlackBlock::Divider],
        };
        assert_eq!(pres.kind(), AdapterKind::Slack);
        assert_eq!(
            serde_json::to_value(&pres).expect("a presentation serialises"),
            serde_json::json!({ "kind": "slack", "blocks": [{ "type": "divider" }] })
        );
    }

    /// Unset optional keys must be absent, not `null` — Slack validates on
    /// presence, so a `"style": null` is a rejected block.
    #[test]
    fn unset_optional_keys_are_omitted_not_null() {
        let block = SlackBlock::Section {
            text: Some(SlackText::PlainText {
                text: "hi".into(),
                emoji: None,
            }),
            fields: None,
        };
        let json = serde_json::to_string(&block).expect("a section serialises");
        assert_eq!(
            json,
            r#"{"type":"section","text":{"type":"plain_text","text":"hi"}}"#
        );
        assert!(!json.contains("null"));
    }
}

// =============================================================================
// Adapter connection
// =============================================================================

/// A configured, credentialed installation of an Adapter.
#[derive(Debug, Clone)]
pub struct AdapterConnection {
    pub id: String,
    pub adapter_kind: AdapterKind,
    pub connection_kind: String,
    pub scope_value: String,
    pub credential_ref: Option<String>,
    pub webhook_secret: Option<String>,
    /// Adapter-specific transport metadata: each Adapter's own flat bag, so the
    /// table needs no per-Adapter columns.
    ///
    /// `slack_workspace_id` also builds `TaskBinding::subject_key` for DM
    /// replies, routing inbound webhook events back to the originating Task.
    pub transport_config: serde_json::Value,
    pub created_by: String,
    pub scope: Scope,
    pub owner_user_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// =============================================================================
// Binding
// =============================================================================

/// The live tie between a Task and an external subject reached through an
/// Adapter — a GitHub PR, a Slack thread, or a Linear issue.
#[derive(Debug, Clone)]
pub struct TaskBinding {
    pub id: String,
    pub task_id: String,
    pub adapter_kind: AdapterKind,
    pub subject_kind: String,
    pub subject_key: String,
    pub connection_id: String,
    pub loop_id: Option<String>,
    pub destination_config: serde_json::Value,
    pub metadata: serde_json::Value,
    pub last_external_event_id: Option<String>,
    /// The direction the Task and external subject met. Responder is
    /// [`ADAPTER_EVENT_BINDING_ORIGIN`]; everything else is a producer —
    /// routing negates the responder rather than matching a list.
    pub origin: String,
    pub created_at: DateTime<Utc>,
    pub last_event_at: DateTime<Utc>,
    /// Display string for the connected subject's external status.
    pub external_status: Option<String>,
    /// Normalized status bucket (GitHub: open|merged|closed|draft; Linear:
    /// state.type).
    pub external_state_kind: Option<String>,
    /// Full status detail (title, url, state id/name, merged flag, …).
    pub external_status_raw: Option<serde_json::Value>,
    /// Latest PR review verdict (GitHub: approved | `changes_requested` |
    /// `review_required` | dismissed | commented). `None` until a review is
    /// recorded.
    pub external_review_decision: Option<String>,
    /// GitHub's coarse mergeable state (clean | dirty | blocked | behind |
    /// unstable | `has_hooks` | draft); `dirty` means merge conflicts. `None`
    /// until computed, or where it does not apply.
    pub external_mergeable_state: Option<String>,
    /// When [`Self::external_status`] was last refreshed.
    pub external_status_updated_at: Option<DateTime<Utc>>,
    /// Whether this binding still routes inbound events. Flipped to `false`
    /// when the Task closes, which is terminal. Inbound routing skips inactive
    /// bindings; a reader that labels a Task does not filter on this, so a
    /// closed Task keeps its adapter label.
    pub active: bool,
    /// When this binding was deactivated; `None` while active.
    pub deactivated_at: Option<DateTime<Utc>>,
}
