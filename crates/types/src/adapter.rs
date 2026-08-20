//! Adapter domain types: the identity of an Adapter, the contexts it is
//! invoked with, the messages that cross its boundary in either direction,
//! and the values that cross its delivery seam.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::access_control::Scope;

/// Canonical external adapter identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum AdapterKind {
    Slack,
    Telegram,
    Github,
    GithubPr,
    Linear,
    Webhook,
}

impl AdapterKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Slack => "slack",
            Self::Telegram => "telegram",
            Self::Github => "github",
            Self::GithubPr => "github_pr",
            Self::Linear => "linear",
            Self::Webhook => "webhook",
        }
    }

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
}

impl std::fmt::Display for AdapterKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// =============================================================================
// Adapter contexts
// =============================================================================

/// Adapter-facing subject context.
///
/// This is synthesized from a task binding before invoking Adapter-specific
/// outbound and context-fetching code.
#[derive(Debug, Clone)]
pub struct BindingContext {
    pub binding_id: String,
    pub loop_id: Option<String>,
    pub task_id: String,
    pub subject_kind: String,
    pub subject_key: String,
    pub destination_config: serde_json::Value,
    pub metadata: serde_json::Value,
    pub last_external_event_id: Option<String>,
    pub last_message_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

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
/// Serialized only into `adapter_delivery.payload`, never into an API response:
/// an Adapter that routes inbound messages through the durable Adapter delivery
/// seam (`delivery`) stores the parsed message in the record's
/// payload, so the record carries everything needed to redo the routing after a
/// restart.
/// Re-deriving it would mean re-parsing a webhook body the process no longer
/// has. Every field is owned data for the same reason — a message that
/// referenced process memory would not be durable work.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InboundMessage {
    /// Adapter type.
    pub adapter_kind: AdapterKind,
    /// Source identifier (Slack channelId, email address).
    pub source_id: String,
    /// Thread ID if this is a reply (Slack thread_ts, Email In-Reply-To).
    pub thread_id: Option<String>,
    /// New thread ID for the message itself (Slack message ts, Email Message-ID).
    pub message_thread_id: String,
    /// Resolved user ID (system user).
    pub user_id: String,
    /// The handle the external system knows the writer by — a GitHub login, a
    /// Slack display name, a Linear actor — recorded verbatim and never
    /// resolved to a Moatless identity (`user_id` holds that, and on most
    /// adapters it is the subscription's owner rather than this writer).
    ///
    /// `#[serde(default)]` because the payload outlives the process that
    /// admitted it: during a rolling deploy the new build executes a record the
    /// old build wrote, and that record names no author.
    #[serde(default)]
    pub author: Option<String>,
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
    /// Source task id when the inbound event was authored by a moatless task
    /// (e.g. the GitHub PR / Linear adapters parse this from the `Moatless task:`
    /// footer link `<frontend_url>/tasks/<id>` on the comment body). Routing uses
    /// this to skip delivery back to the authoring task while still fanning out
    /// to other bindings on the same subject.
    pub source_task_id: Option<String>,
    /// Who this event is for. See [`Audience`].
    ///
    /// `#[serde(default)]` because the payload outlives the process that
    /// admitted it: during a rolling deploy the new build executes records the
    /// old build wrote, and those carry no audience. The default is
    /// [`Audience::Everyone`], which is what routing did before this field
    /// existed.
    #[serde(default)]
    pub audience: Audience,
}

/// Who an inbound event is for, as stated by the adapter that parsed it.
///
/// Classification stays with the adapter because only the adapter knows its own
/// event shapes — it is a match over values the adapter already owns. Turning
/// an audience into the bindings that receive the event is the router's job,
/// through the binding layer's `resolve_recipients`. An adapter with
/// no audience distinction to make states [`Audience::Everyone`] for every
/// event, which is what it already does.
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
    /// produced the subject — falling back to every active binding when neither
    /// is bound, on the same grounds as [`Audience::SubjectProducer`]: a thread
    /// scoped to nobody is not a reason to deliver to nobody. A task spawned
    /// from the subject hears only the threads it started; the producer must
    /// address every thread on its own work, so it receives replies inside all
    /// of them.
    ParentAuthor {
        /// Identifier of the event being replied to, in the same namespace as
        /// [`InboundMessage::external_event_id`]. `None` when the event starts
        /// its own thread and so names no parent — nobody authored a parent,
        /// leaving the producer as the only recipient, or the whole subject
        /// where no producer is bound either.
        parent_external_event_id: Option<String>,
    },
}

/// Resolved file attachment for outbound adapter messages.
#[derive(Clone, Debug)]
pub struct OutboundFile {
    pub filename: String,
    pub mime_type: String,
    pub data: Vec<u8>,
}

// =============================================================================
// Adapter-native presentation
// =============================================================================

/// Adapter-native rich rendering of an outbound message body.
///
/// [`OutboundReply::text`] stays mandatory and portable: every adapter sends it,
/// Slack uses it for the notification, and the binding layer records it as the
/// task-timeline entry. A presentation is the optional richer form, for the
/// adapters that have one. An adapter ignores a presentation addressed to
/// another kind, the way Slack already ignores `footer`.
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
/// Typed rather than a raw `Value` so a malformed payload fails where it is
/// entered, with a field path, instead of reaching Slack and returning a bare
/// `invalid_blocks` with the message silently unposted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SlackBlock {
    /// Large bold title. Slack accepts only `plain_text` here.
    Header { text: SlackPlainText },
    /// Body text, optionally with a two-column `fields` array.
    Section {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<SlackText>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fields: Option<Vec<SlackText>>,
    },
    /// Standard Markdown that Slack translates, headings and tables included.
    Markdown { text: String },
    /// Tabular data. Cells are rich text, so they can carry live links.
    Table { rows: Vec<Vec<SlackTableCell>> },
    /// Small muted line, typically a footer.
    Context { elements: Vec<SlackText> },
    Divider,
    Image { image_url: String, alt_text: String },
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

/// Where a reply came from, rendered as a footer by adapters that link back to
/// the task themselves.
///
/// Linear and GitHub PR do not use this — their comment bodies are Markdown, so
/// they carry the link in [`OutboundReply::footer`] via `render_footer`. Slack
/// mrkdwn has no `[text](url)`, which is why the pieces travel structured and
/// the adapter renders them.
///
/// Every field but `url` is droppable: a missing workspace or task name omits
/// its segment rather than failing the send.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TaskProvenance {
    /// `{frontend_url}/tasks/{task_id}`.
    pub url: String,
    pub task_name: Option<String>,
    pub workspace_name: Option<String>,
    pub origin: TaskOrigin,
}

impl TaskProvenance {
    /// Text for the link itself: the task's name, or a generic fallback — the
    /// same rule `render_footer` uses for Linear and GitHub.
    pub fn label(&self) -> &str {
        self.task_name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("View task")
    }

    /// The segments that follow the link, in order, skipping what is unknown.
    /// Adapters join these with their own separator so one adapter's markup
    /// never leaks into another's.
    pub fn details(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(ws) = self
            .workspace_name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            out.push(ws.to_string());
        }
        match &self.origin {
            TaskOrigin::Loop { name } => out.push(format!("loop: {name}")),
            TaskOrigin::StartedBy { login } => out.push(format!("started by {login}")),
            TaskOrigin::Unknown => {}
        }
        out
    }
}

/// What set a task going.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TaskOrigin {
    /// Fired by a Loop, named by `loops.name`.
    Loop { name: String },
    /// Started by a person, named by their moatless login.
    StartedBy { login: String },
    /// Neither is known — the task predates provenance, or both lookups failed.
    Unknown,
}

/// Outbound reply to send back to an external target.
#[derive(Debug)]
pub struct OutboundReply {
    pub text: String,
    /// Provenance footer for adapters that render the task link themselves
    /// (Slack). `None` leaves the reply without one.
    pub provenance: Option<TaskProvenance>,
    /// Footer suffix appended to comment bodies by the Linear/GitHub adapters
    /// (e.g. `"\n\n---\nMoatless task: [Task name](https://app/tasks/<id>)"`).
    /// Built by the binding layer's `task_comment_footer`. The footer URL
    /// doubles as the inbound echo-filter source. `None` for providers that
    /// don't carry a footer.
    pub footer: Option<String>,
    /// Preferred system user whose Adapter credentials should author this
    /// reply — the owning task's `owner_user_id`. Providers that resolve a
    /// per-user credential (currently GitHub) prefer this over binding/trigger
    /// identities so a human-owned task replies under the human, and a
    /// trigger-created automation task replies under its run-as bot. `None`
    /// when no task owner is in scope (non-reply sends) or for providers that
    /// don't use it.
    pub author_user_id: Option<String>,
    pub is_error: bool,
    pub files: Vec<OutboundFile>,
    /// Adapter-native rich rendering of this reply, when the caller supplied
    /// one. `None` reproduces the pre-presentation behaviour exactly.
    pub presentation: Option<OutboundPresentation>,
}

// =============================================================================
// Adapter delivery
// =============================================================================
//
// A destination names where a reply lands; a payload carries everything a
// worker needs to redo inbound routing after the admitting process is gone.

/// Result of posting to a provider-specific destination by raw ID.
///
/// Returned by `AdapterDirectDestination::post_to_destination` and used by
/// callers to build a matching `TaskBinding` row when the post is associated
/// with a task.
#[derive(Debug, Clone)]
pub struct DestinationPostResult {
    /// Provider-specific external message ID (e.g. Slack message `ts`).
    pub message_external_id: String,
    /// Provider-specific binding `subject_kind`.
    pub subject_kind: &'static str,
    /// Binding `subject_key` matching the format produced during inbound
    /// routing for this provider.
    pub subject_key: String,
    /// Binding `destination_config` JSON used by `send_reply_for_binding`.
    pub destination_config: serde_json::Value,
}

/// Provider-owned destination data for a task binding.
#[derive(Debug, Clone)]
pub struct BindingDestination {
    /// Binding `subject_kind` (for example, a provider-defined thread kind).
    pub subject_kind: &'static str,
    /// Stable provider-defined lookup key for the bound subject.
    pub subject_key: String,
    /// Provider-defined JSON needed to send future replies.
    pub destination_config: serde_json::Value,
    /// Optional cursor seeded when the binding is created from an outbound send.
    pub last_external_event_id: Option<String>,
}

impl BindingDestination {
    pub fn new(
        subject_kind: &'static str,
        subject_key: String,
        destination_config: serde_json::Value,
    ) -> Self {
        Self {
            subject_kind,
            subject_key,
            destination_config,
            last_external_event_id: None,
        }
    }

    pub fn with_last_external_event_id(mut self, external_id: impl Into<String>) -> Self {
        self.last_external_event_id = Some(external_id.into());
        self
    }
}

/// Everything `route_inbound_message` needs, in a form that outlives the
/// process that admitted it.
///
/// The connection and the Loop are referenced by id rather than embedded:
/// they are durable rows already, and a connection carries credentials that
/// have no business being copied into a queue row that outlives the request by
/// days.
///
/// Fields may only ever be *added*, and only with `#[serde(default)]`. During a
/// rolling deploy the new build executes records the old build admitted, so a
/// payload that stops deserializing is a payload whose work goes terminal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundDeliveryPayload {
    pub connection_id: String,
    pub loop_id: String,
    pub subject_kind: String,
    pub subject_key: String,
    pub destination_config: serde_json::Value,
    /// The sender's own event name, carried only so the webhook outcome
    /// metric keeps the labels it had while routing ran on the request path.
    /// Empty for senders that never labelled it.
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
    /// OAuth client_id from the Linear OAuth2 app registration.
    pub oauth_client_id: Option<String>,
    /// OAuth client_secret from the Linear OAuth2 app registration.
    pub oauth_client_secret: Option<String>,
    /// OAuth redirect_uri registered on the Linear OAuth2 app.
    pub oauth_callback_url: Option<String>,
    /// API host every Linear client built by this adapter talks to, GraphQL and
    /// OAuth token exchange alike. `None` means Linear itself.
    ///
    /// This is the Adapter's injection seam: `LinearAgentApi` is constructed
    /// deep inside webhook and worker paths that hold only an `AppState`, so
    /// pointing the clients somewhere else has to be something an `AppState`
    /// carries. A test builds its own, which is why it cannot be a
    /// process-global. Env: `LINEAR_API_BASE_URL`.
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
                    text: Some(SlackText::Mrkdwn { text: "*7 apps*".into() }),
                    fields: None,
                },
                serde_json::json!({
                    "type": "section",
                    "text": { "type": "mrkdwn", "text": "*7 apps*" }
                }),
            ),
            (
                SlackBlock::Markdown { text: "### Heading".into() },
                serde_json::json!({ "type": "markdown", "text": "### Heading" }),
            ),
            (
                SlackBlock::Divider,
                serde_json::json!({ "type": "divider" }),
            ),
            (
                SlackBlock::Context {
                    elements: vec![SlackText::Mrkdwn { text: "93 to promote".into() }],
                },
                serde_json::json!({
                    "type": "context",
                    "elements": [{ "type": "mrkdwn", "text": "93 to promote" }]
                }),
            ),
        ];

        for (block, expected) in cases {
            assert_eq!(serde_json::to_value(&block).expect("a block serialises"), expected);
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
                        style: Some(SlackTextStyle { bold: Some(true), ..Default::default() }),
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
            text: Some(SlackText::PlainText { text: "hi".into(), emoji: None }),
            fields: None,
        };
        let json = serde_json::to_string(&block).expect("a section serialises");
        assert_eq!(json, r#"{"type":"section","text":{"type":"plain_text","text":"hi"}}"#);
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
    /// Adapter-specific transport metadata. Each Adapter stores its own flat
    /// bag of identifiers and config here, so the table needs no per-Adapter
    /// columns.
    ///
    /// Conventions in use today:
    /// - `source`: `"env"` | `"oauth"` — where the credential came from.
    /// - `slack_workspace_id`: e.g. `"T0000000002"` — the Slack workspace ID.
    ///   Used to build `TaskBinding::subject_key` for DM replies so that
    ///   inbound webhook events route back to the originating Task.
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
    /// The direction in which the Task and the external subject met. The
    /// responder direction is [`ADAPTER_EVENT_BINDING_ORIGIN`]; every other
    /// value is a producer, so routing classifies the role by negating the
    /// responder rather than by matching a list.
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
    /// Latest PR review verdict (GitHub: approved | changes_requested |
    /// review_required | dismissed | commented). `None` until a review is
    /// recorded.
    pub external_review_decision: Option<String>,
    /// GitHub's coarse mergeable state (clean | dirty | blocked | behind |
    /// unstable | has_hooks | draft); `dirty` means merge conflicts. `None`
    /// until computed, or where it does not apply.
    pub external_mergeable_state: Option<String>,
    /// When [`Self::external_status`] was last refreshed.
    pub external_status_updated_at: Option<DateTime<Utc>>,
    /// Whether this binding still routes inbound events. Flipped to `false`
    /// when the Task closes, which is terminal. Inbound routing skips inactive
    /// bindings; the admin Task Connections view still shows them, flagged.
    pub active: bool,
    /// When this binding was deactivated; `None` while active.
    pub deactivated_at: Option<DateTime<Utc>>,
}
