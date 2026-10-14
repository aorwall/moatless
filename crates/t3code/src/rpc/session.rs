use crate::authentication::viewer_scope;
use crate::rpc::config::now_rfc3339;
use crate::rpc::envelope::ClientFrame;
use crate::rpc::envelope::RequestId;
use crate::rpc::envelope::connection_defect;
use crate::rpc::envelope::interrupted_exit;
use crate::rpc::envelope::valid_request_id;
use crate::rpc::metrics::request_root_span;
use crate::skills;
use crate::state::UiRpcState;
use crate::subscription::DetachedSet;
use crate::subscription::StreamSet;
use moatless_types::ui_rpc::UiRpcAuthIdentity;
use serde_json::Value;
use serde_json::json;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;
use tokio::sync::mpsc;

pub(crate) const MAX_MESSAGES_PER_FRAME: usize = 128;
pub(crate) const MAX_ACTIVE_STREAMS: usize = 64;

/// How many entries a workspace search answers with when the client asks for no
/// particular number.
///
/// The picker renders a scrolling list; a hundred is past what anyone reads and
/// well inside what one gRPC round trip returns.
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
/// nothing, costing a full scan to say so; below this the answer is empty
/// without a query.
pub(crate) const SEARCH_THREADS_MIN_QUERY: usize = 2;

/// The most `cwd`s one connection keeps skill listings for.
///
/// Every `cwd` addressed is republished on every provider refresh; past this
/// the oldest is dropped, and the composer asks for it again if the person goes
/// back.
pub(crate) const MAX_WORKSPACE_SNAPSHOTS: usize = 32;

/// The most requests one connection answers off its frame loop at once.
///
/// Past this a request is answered on the loop instead (where every request was
/// answered before any detached); the cap bounds work in flight, and reaching
/// it costs latency, not an answer.
pub(crate) const MAX_DETACHED_REQUESTS: usize = 32;

/// The `subscribeServerConfig` request this connection has open, when it has
/// one.
///
/// A refreshed provider list reaches the client's state through that stream and
/// nowhere else — the reply to `server.refreshProviders` only tells the
/// composer whether to ask again.
#[derive(Clone, Debug, Default)]
pub(crate) struct ConfigStream(Arc<Mutex<Option<RequestId>>>);

impl ConfigStream {
    pub(crate) fn opened(&self) -> Option<RequestId> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn open(&self, id: &RequestId) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(id.clone());
    }

    /// Forget the stream `id` opened. Another id closes nothing: the client
    /// interrupts each of its subscriptions and only one of them is this.
    pub(crate) fn close(&self, id: &RequestId) {
        let mut opened = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if opened.as_ref() == Some(id) {
            *opened = None;
        }
    }
}

/// The per-`cwd` skill listings this connection has resolved, oldest first, in
/// the shape `ServerProviderWorkspaceSnapshot` decodes from.
///
/// Kept because a `providerStatuses` event replaces the whole provider list;
/// publishing only the just-asked `cwd` would empty the composer of every
/// thread already opened.
#[derive(Clone, Debug, Default)]
pub(crate) struct WorkspaceSnapshots(Arc<Mutex<Vec<(String, Value)>>>);

impl WorkspaceSnapshots {
    /// Record one `cwd`'s skills, replacing what this connection knew of it.
    pub(crate) fn remember(&self, cwd: &str, snapshot: Value) {
        let mut snapshots = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        snapshots.retain(|(known, _)| known != cwd);
        snapshots.push((cwd.to_string(), snapshot));
        if snapshots.len() > MAX_WORKSPACE_SNAPSHOTS {
            snapshots.remove(0);
        }
    }

    pub(crate) fn published(&self) -> Vec<Value> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|(_, snapshot)| snapshot.clone())
            .collect()
    }
}

/// The session state a request may reach without holding the connection.
///
/// Both halves are behind a lock so `server.refreshProviders` can be answered
/// off the frame loop; a guard is held for a single read or write, never across
/// an await, and poisoning is recovered rather than propagated.
#[derive(Clone, Debug, Default)]
pub(crate) struct SharedSession {
    pub(crate) config_stream: ConfigStream,
    pub(crate) workspace_snapshots: WorkspaceSnapshots,
    /// Fixed when the socket opens; see [`UiRpcConfig::protocol_for`].
    ///
    /// [`UiRpcConfig::protocol_for`]: crate::config::UiRpcConfig::protocol_for
    pub(crate) protocol: crate::config::OrchestrationProtocol,
}

#[derive(Debug)]
pub(crate) struct ProtocolSession {
    pub(crate) streams: StreamSet,
    /// The requests this connection has running elsewhere. See
    /// [`crate::rpc::dispatch::routing::detach`] for which ones go here.
    pub(crate) detached: DetachedSet,
    pub(crate) shared: SharedSession,
}

impl ProtocolSession {
    pub(crate) fn new(frames: mpsc::Sender<Value>) -> Self {
        Self {
            streams: StreamSet::new(frames.clone()),
            detached: DetachedSet::new(frames),
            shared: SharedSession::default(),
        }
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
        // The frame is in this connection's hands from here, so this is where a
        // request batched behind another starts waiting. See `queued_ms` below.
        let received = std::time::Instant::now();
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
                    self.detached.interrupt(&request_id);
                    self.shared.config_stream.close(&request_id);
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
                    // What this request waited for behind its own batch. The
                    // wait a *frame* spends behind another frame is not visible
                    // here — see [`crate::rpc::metrics::CONNECTION_BLOCKED`].
                    span.record("queued_ms", received.elapsed().as_secs_f64() * 1000.0);
                    if let Some(frames) =
                        self.answer(id, tag, payload, state, identity, &span).await
                    {
                        output.frames.extend(frames);
                    }
                }
            }
        }
        output
    }
}
/// Read the skills of the Workspace a `cwd` names and record them for this
/// connection.
///
/// A `cwd` this surface never issued, or one whose Workspace this viewer can't
/// be shown, records nothing — the composer falls back to the provider's own
/// empty skill list.
pub(crate) async fn resolve_workspace_skills(
    state: &UiRpcState,
    identity: &UiRpcAuthIdentity,
    cwd: &str,
    snapshots: &WorkspaceSnapshots,
) {
    let viewer = viewer_scope(identity);
    let Some(workspace_id) = skills::workspace_of(state.reads.conversations(), &viewer, cwd).await
    else {
        return;
    };
    let listed = state
        .skills
        .list_for_workspace(&viewer, identity.user_id, &workspace_id)
        .await;
    snapshots.remember(cwd, skills::snapshot(cwd, &now_rfc3339(), &listed));
}
