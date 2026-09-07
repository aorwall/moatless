use crate::authentication::viewer_scope;
use crate::rpc::config::now_rfc3339;
use crate::rpc::frames::ClientFrame;
use crate::rpc::frames::RequestId;
use crate::rpc::frames::connection_defect;
use crate::rpc::frames::interrupted_exit;
use crate::rpc::frames::valid_request_id;
use crate::rpc::metrics::record_request;
use crate::rpc::metrics::request_root_span;
use crate::skills;
use crate::state::UiRpcState;
use crate::subscription::StreamSet;
use moatless_types::ui_rpc::UiRpcAuthIdentity;
use serde_json::Value;
use serde_json::json;
use tokio::sync::mpsc;
use tracing::Instrument;

pub(crate) const MAX_MESSAGES_PER_FRAME: usize = 128;
pub(crate) const MAX_ACTIVE_STREAMS: usize = 64;

/// How many entries a workspace search answers with when the client asks for no
/// particular number.
///
/// The picker renders a scrolling list and the person picks from the top of it;
/// a hundred is past what anyone reads and well inside what one gRPC round trip
/// returns.
pub(crate) const SEARCH_ENTRIES_LIMIT: usize = 100;

/// The most entries one search may ask for, whatever the client requests.
///
/// The limit reaches the Sandbox, which walks the tree to satisfy it, so an
/// unbounded request is a way to make a pod work arbitrarily hard from a
/// keystroke.
pub(crate) const SEARCH_ENTRIES_MAX: usize = 200;

/// How many conversations one search answers with.
///
/// One row per Task, so this is fifty conversations rather than fifty messages
/// — the palette shows a handful and this leaves room to rank.
pub(crate) const SEARCH_THREADS_LIMIT: i64 = 50;

/// The shortest query worth running against every Message body.
///
/// A one-character search matches most of the corpus and tells the person
/// nothing, and it costs a full scan to say so. Below this the answer is empty
/// without a query.
pub(crate) const SEARCH_THREADS_MIN_QUERY: usize = 2;

/// The most `cwd`s one connection keeps skill listings for.
///
/// A person browsing threads all afternoon addresses a new `cwd` each time, and
/// every one of them is republished on every provider refresh. Past this the
/// oldest is dropped; the composer asks for it again if they go back.
pub(crate) const MAX_WORKSPACE_SNAPSHOTS: usize = 32;

#[derive(Debug)]
pub(crate) struct ProtocolSession {
    pub(crate) streams: StreamSet,
    /// The open `subscribeServerConfig` request, when the client has one. A
    /// refreshed provider list reaches the client's state through that stream
    /// and nowhere else — the reply to `server.refreshProviders` only tells the
    /// composer whether to ask again.
    pub(crate) config_stream: Option<RequestId>,
    /// The per-`cwd` skill listings this connection has resolved, oldest first,
    /// in the shape `ServerProviderWorkspaceSnapshot` decodes from.
    ///
    /// Kept because a `providerStatuses` event replaces the whole provider
    /// list: publishing only the `cwd` that was just asked for would empty the
    /// composer of every thread the person had already opened.
    pub(crate) workspace_snapshots: Vec<(String, Value)>,
}

impl ProtocolSession {
    pub(crate) fn new(frames: mpsc::Sender<Value>) -> Self {
        Self {
            streams: StreamSet::new(frames),
            config_stream: None,
            workspace_snapshots: Vec::new(),
        }
    }

    /// Record one `cwd`'s skills, replacing what this connection knew of it.
    pub(crate) fn remember_workspace_snapshot(&mut self, cwd: &str, snapshot: Value) {
        self.workspace_snapshots.retain(|(known, _)| known != cwd);
        self.workspace_snapshots.push((cwd.to_string(), snapshot));
        if self.workspace_snapshots.len() > MAX_WORKSPACE_SNAPSHOTS {
            self.workspace_snapshots.remove(0);
        }
    }

    pub(crate) fn published_workspace_snapshots(&self) -> Vec<Value> {
        self.workspace_snapshots
            .iter()
            .map(|(_, snapshot)| snapshot.clone())
            .collect()
    }
}

#[derive(Debug, Default)]
pub(crate) struct ProtocolOutput {
    pub(crate) frames: Vec<Value>,
    pub(crate) eof: bool,
}

impl ProtocolSession {
    pub(crate) async fn handle_json(
        &mut self,
        bytes: &[u8],
        state: &UiRpcState,
        identity: &UiRpcAuthIdentity,
    ) -> ProtocolOutput {
        let value: Value = match serde_json::from_slice(bytes) {
            Ok(value) => value,
            Err(error) => {
                return ProtocolOutput {
                    frames: vec![connection_defect(
                        "malformed_json",
                        format!("Malformed JSON frame: {error}"),
                    )],
                    eof: false,
                };
            }
        };

        let values = match value {
            Value::Array(values) => values,
            value => vec![value],
        };
        if values.len() > MAX_MESSAGES_PER_FRAME {
            return ProtocolOutput {
                frames: vec![connection_defect(
                    "frame_too_large",
                    format!("A frame may contain at most {MAX_MESSAGES_PER_FRAME} messages"),
                )],
                eof: true,
            };
        }

        let mut output = ProtocolOutput::default();
        for value in values {
            let frame: ClientFrame = match serde_json::from_value(value) {
                Ok(frame) => frame,
                Err(error) => {
                    output.frames.push(connection_defect(
                        "malformed_envelope",
                        format!("Malformed RPC envelope: {error}"),
                    ));
                    continue;
                }
            };

            match frame {
                ClientFrame::Ping => output.frames.push(json!({ "_tag": "Pong" })),
                ClientFrame::Eof => {
                    output.eof = true;
                    break;
                }
                ClientFrame::Ack { request_id } => {
                    if !valid_request_id(request_id.as_str()) {
                        output.frames.push(connection_defect(
                            "invalid_ack_id",
                            "Ack requestId is not a decimal string",
                        ));
                        continue;
                    }
                    self.streams.ack(&request_id);
                }
                ClientFrame::Interrupt { request_id } => {
                    if !valid_request_id(request_id.as_str()) {
                        output.frames.push(connection_defect(
                            "invalid_interrupt_id",
                            "Interrupt requestId is not a decimal string",
                        ));
                        continue;
                    }
                    self.streams.interrupt(&request_id);
                    if self.config_stream.as_ref() == Some(&request_id) {
                        self.config_stream = None;
                    }
                    output.frames.push(interrupted_exit(&request_id));
                }
                ClientFrame::Request {
                    id,
                    tag,
                    payload,
                    headers,
                    trace_id,
                    span_id,
                    sampled,
                } => {
                    // Not consumed: the sibling diagnostics spec, not this one.
                    drop(headers);
                    let span =
                        request_root_span(&tag, trace_id.as_deref(), span_id.as_deref(), sampled);
                    let started = std::time::Instant::now();
                    let frames = self
                        .handle_request(id, tag.clone(), payload, state, identity)
                        .instrument(span.clone())
                        .await;
                    record_request(&span, &tag, &frames, started.elapsed(), identity);
                    output.frames.extend(frames);
                }
            }
        }
        output
    }
}
impl ProtocolSession {
    /// Read the skills of the Workspace a `cwd` names and record them for this
    /// connection.
    ///
    /// A `cwd` this surface never issued, or one whose Workspace this viewer
    /// cannot be shown, records nothing: the composer then falls back to the
    /// provider's own (empty) skill list rather than being told the Workspace
    /// exists.
    pub(crate) async fn resolve_workspace_skills(
        &mut self,
        state: &UiRpcState,
        identity: &UiRpcAuthIdentity,
        cwd: &str,
    ) {
        let viewer = viewer_scope(identity);
        let Some(workspace_id) =
            skills::workspace_of(state.conversations.as_ref(), &viewer, cwd).await
        else {
            return;
        };
        let listed = state
            .skills
            .list_for_workspace(&viewer, identity.user_id, &workspace_id)
            .await;
        let snapshot = skills::snapshot(cwd, &now_rfc3339(), &listed);
        self.remember_workspace_snapshot(cwd, snapshot);
    }
}
