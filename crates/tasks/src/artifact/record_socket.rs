//! The socket a served page subscribes to its Artifact's records on, at
//! [`PAGE_SOCKET_ROUTE`], authenticated by the page token like the record
//! routes.
//!
//! Frames take UI-RPC's shapes (`Request`, `Chunk`, `Exit`, `Interrupt`,
//! `Ping`/`Pong`), so a page reads them the way T3 reads its own. A wake from
//! [`super::record_watch::RecordWatch`] reads the change log after the
//! socket's cursor, so a write on any replica reaches every subscriber.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::time::{Instant, interval_at};
use tracing::{debug, warn};

use super::error::ArtifactError;
use super::models::{Artifact, ArtifactRecord};
use super::operations::Artifacts;
use super::records::{page_reader, readable_by, to_response};
use crate::task::operations::TaskCommands;

/// The page's record socket; `{token}` is the page token.
///
/// `data_script.js` holds a copy of this literal, and the auth middleware and
/// the request span's token redaction (`moatless_backend::http_trace`) match
/// every `/api/artifact-data/` path; rename them together.
pub const PAGE_SOCKET_ROUTE: &str = "/api/artifact-data/{token}/ws";

/// The one method the socket answers: stream the records under a prefix.
const SUBSCRIBE: &str = "artifact.records.subscribe";
/// How often the socket checks that its reader can still read the Artifact
/// and that the latest publish still accepts data. Also when it rereads the
/// change log should this replica's watch have stalled.
const RECHECK_INTERVAL: Duration = Duration::from_secs(15);
const MAX_SUBSCRIPTIONS: usize = 16;
/// A page sends only small control frames.
const MAX_INBOUND_BYTES: usize = 16 * 1024;
/// Records per `Chunk`, so a snapshot of a full Artifact is many frames
/// rather than one of hundreds of megabytes.
const RECORDS_PER_CHUNK: usize = 100;

/// Upgrade a page's request to its record socket, once its token names a
/// reader who can still read the Artifact.
pub async fn page_socket(
    State(tasks): State<Arc<TaskCommands>>,
    State(artifacts): State<Arc<Artifacts>>,
    Path(token): Path<String>,
    upgrade: WebSocketUpgrade,
) -> Response {
    let (claims, row) = match page_reader(&tasks, &artifacts, &token).await {
        Ok(reader) => reader,
        Err(error) => return error.into_response(),
    };
    let connection = Connection {
        tasks,
        artifacts,
        token,
        viewer: claims.sub,
        expires_at: claims.exp,
        row,
        subscriptions: Vec::new(),
    };
    upgrade
        .max_message_size(MAX_INBOUND_BYTES)
        .on_upgrade(move |socket| connection.serve(socket))
}

#[derive(Debug, Deserialize)]
#[serde(tag = "_tag")]
enum ClientFrame {
    Request {
        id: Value,
        tag: String,
        #[serde(default)]
        payload: Value,
    },
    Interrupt {
        #[serde(rename = "requestId")]
        request_id: Value,
    },
    Ping,
    Eof,
    /// `Ack` and anything newer: nothing here waits on them.
    #[serde(other)]
    Other,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubscribePayload {
    prefix: Option<String>,
}

/// One `artifact.records.subscribe` request, and what its page holds.
#[derive(Debug)]
struct Subscription {
    id: Value,
    prefix: Option<String>,
    /// The `record_seq` the page has every change up to.
    cursor: i64,
    /// The keys the page holds a record for, so a record it may no longer read
    /// is removed from it without naming keys it never saw.
    shown: HashSet<String>,
    /// A resync failed, so the page may hold records the current rules hide.
    stale: bool,
}

/// What a subscription's page is sent for the changes up to a head.
#[derive(Debug)]
enum Advance {
    Nothing,
    /// The log no longer reaches back to its cursor: read every record again.
    Resync,
    Changes {
        records: Vec<ArtifactRecord>,
        removed: Vec<String>,
    },
}

impl Subscription {
    fn covers(&self, key: &str) -> bool {
        self.prefix
            .as_deref()
            .is_none_or(|prefix| key.starts_with(prefix))
    }

    /// Move to `head` over `changes`, the log after a cursor at or below this
    /// one's, with `current` holding the record under each changed key that
    /// still has one.
    fn advance(
        &mut self,
        head: i64,
        changes: &[(i64, String)],
        current: &HashMap<&str, &ArtifactRecord>,
        readable: impl Fn(&ArtifactRecord) -> bool,
    ) -> Advance {
        if self.stale {
            return Advance::Resync;
        }
        if head <= self.cursor {
            return Advance::Nothing;
        }
        let oldest = changes.first().map_or(head + 1, |(seq, _)| *seq);
        if oldest > self.cursor + 1 {
            return Advance::Resync;
        }
        let mut keys: Vec<&str> = Vec::new();
        for (_, key) in changes.iter().filter(|(seq, _)| *seq > self.cursor) {
            if self.covers(key) {
                keys.retain(|k| k != key);
                keys.push(key);
            }
        }
        self.cursor = head;
        let (mut records, mut removed) = (Vec::new(), Vec::new());
        for key in keys {
            match current.get(key).filter(|record| readable(record)) {
                Some(record) => {
                    self.shown.insert(key.to_string());
                    records.push((*record).clone());
                }
                None if self.shown.remove(key) => removed.push(key.to_string()),
                None => {}
            }
        }
        if records.is_empty() && removed.is_empty() {
            Advance::Nothing
        } else {
            Advance::Changes { records, removed }
        }
    }
}

struct Connection {
    tasks: Arc<TaskCommands>,
    artifacts: Arc<Artifacts>,
    token: String,
    viewer: String,
    /// The page token's `exp`, in Unix seconds.
    expires_at: u64,
    /// The Artifact as of the last access check; its rules decide reads.
    row: Artifact,
    subscriptions: Vec<Subscription>,
}

impl Connection {
    async fn serve(mut self, mut socket: WebSocket) {
        let mut heads = self
            .artifacts
            .record_watch()
            .subscribe(&self.row.artifact_id);
        let mut recheck = interval_at(Instant::now() + RECHECK_INTERVAL, RECHECK_INTERVAL);
        let expiry = tokio::time::sleep(until_unix(self.expires_at));
        tokio::pin!(expiry);
        loop {
            let frames = tokio::select! {
                message = socket.recv() => match message {
                    Some(Ok(Message::Text(text))) => self.on_message(text.as_bytes()).await,
                    Some(Ok(Message::Binary(bytes))) => self.on_message(&bytes).await,
                    Some(Ok(Message::Ping(payload))) => Inbound::Pong(payload),
                    Some(Ok(Message::Pong(_))) => continue,
                    Some(Ok(Message::Close(_)) | Err(_)) | None => break,
                },
                changed = heads.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    Inbound::Frames(self.pull().await)
                }
                _ = recheck.tick() => match self.recheck().await {
                    Ok(frames) => Inbound::Frames(frames),
                    Err(error) if ends_access(&error) => Inbound::End(self.ended(&error)),
                    Err(error) => {
                        warn!(error = %moatless_core::error::causal_chain(&error), artifact_id = %self.row.artifact_id, "Artifact record socket recheck failed");
                        Inbound::Frames(Vec::new())
                    }
                },
                () = &mut expiry => Inbound::End(self.ended(&ArtifactError::Unauthorized)),
            };
            if !send(&mut socket, frames).await {
                break;
            }
        }
        debug!(artifact_id = %self.row.artifact_id, "Artifact record socket closed");
    }

    async fn on_message(&mut self, bytes: &[u8]) -> Inbound {
        let frames = match serde_json::from_slice::<Value>(bytes) {
            Ok(Value::Array(frames)) => frames,
            Ok(frame) => vec![frame],
            Err(error) => return Inbound::Frames(vec![defect(format!("Malformed JSON: {error}"))]),
        };
        let mut out = Vec::new();
        for frame in frames {
            match serde_json::from_value::<ClientFrame>(frame) {
                Ok(ClientFrame::Request { id, tag, payload }) => {
                    out.extend(self.on_request(id, &tag, payload).await);
                }
                Ok(ClientFrame::Interrupt { request_id }) => {
                    self.subscriptions.retain(|s| s.id != request_id);
                    out.push(exit(&request_id, json!({ "_tag": "Interrupt" })));
                }
                Ok(ClientFrame::Ping) => out.push(json!({ "_tag": "Pong" })),
                Ok(ClientFrame::Eof) => return Inbound::End(out),
                Ok(ClientFrame::Other) => {}
                Err(error) => out.push(defect(format!("Malformed RPC envelope: {error}"))),
            }
        }
        Inbound::Frames(out)
    }

    async fn on_request(&mut self, id: Value, tag: &str, payload: Value) -> Vec<Value> {
        if tag != SUBSCRIBE {
            return vec![die(&id, format!("{tag} is not a method of this socket"))];
        }
        if self.subscriptions.iter().any(|s| s.id == id) {
            return vec![die(&id, format!("request {id} is already subscribed"))];
        }
        if self.subscriptions.len() >= MAX_SUBSCRIPTIONS {
            return vec![die(
                &id,
                format!("a page may hold at most {MAX_SUBSCRIPTIONS} subscriptions"),
            )];
        }
        let payload = match payload {
            Value::Null => Ok(SubscribePayload::default()),
            payload => serde_json::from_value(payload),
        };
        let prefix = match payload {
            Ok(payload) => payload.prefix,
            Err(error) => return vec![die(&id, format!("{SUBSCRIBE}: {error}"))],
        };
        let mut subscription = Subscription {
            id,
            prefix,
            cursor: 0,
            shown: HashSet::new(),
            stale: false,
        };
        let frames = match self.snapshot(&mut subscription).await {
            Ok(frames) => frames,
            Err(error) => return vec![failed(&subscription.id, &error)],
        };
        self.subscriptions.push(subscription);
        frames
    }

    /// Every record the subscription's page may read, as `Snapshot` chunks the
    /// last of which is `done`.
    async fn snapshot(&self, subscription: &mut Subscription) -> Result<Vec<Value>, ArtifactError> {
        let id = &self.row.artifact_id;
        // Read before the records, so a write between the two is sent again
        // as a change rather than missed.
        let head = self.artifacts.record_head(id).await?;
        let records = self
            .artifacts
            .list_records(id, None, subscription.prefix.as_deref())
            .await?;
        let records: Vec<_> = records
            .iter()
            .filter(|record| readable_by(&self.row, record, &self.viewer))
            .collect();
        subscription.cursor = head;
        subscription.shown = records.iter().map(|r| r.key.clone()).collect();
        let batches: Vec<_> = records.chunks(RECORDS_PER_CHUNK).collect();
        let last = batches.len().saturating_sub(1);
        if batches.is_empty() {
            return Ok(vec![snapshot_chunk(&subscription.id, &[], true)]);
        }
        Ok(batches
            .iter()
            .enumerate()
            .map(|(i, batch)| snapshot_chunk(&subscription.id, batch, i == last))
            .collect())
    }

    /// The changes logged since the furthest-behind subscription's cursor.
    /// A failed read is logged and leaves every cursor where it was, so the
    /// next wake reads the same changes again.
    async fn pull(&mut self) -> Vec<Value> {
        let Some(after) = self.subscriptions.iter().map(|s| s.cursor).min() else {
            return Vec::new();
        };
        match self.read_changes(after).await {
            Ok(frames) => frames,
            Err(error) => {
                warn!(error = %moatless_core::error::causal_chain(&error), artifact_id = %self.row.artifact_id, "Failed to read artifact record changes");
                Vec::new()
            }
        }
    }

    async fn read_changes(&mut self, after: i64) -> Result<Vec<Value>, ArtifactError> {
        let id = &self.row.artifact_id;
        let (head, changes) = self.artifacts.record_changes(id, after).await?;
        let mut keys: Vec<String> = changes
            .iter()
            .map(|(_, key)| key)
            .filter(|key| self.subscriptions.iter().any(|s| s.covers(key)))
            .cloned()
            .collect();
        keys.sort();
        keys.dedup();
        let stored = if keys.is_empty() {
            Vec::new()
        } else {
            self.artifacts.find_records(id, &keys).await?
        };
        let current: HashMap<&str, &ArtifactRecord> =
            stored.iter().map(|r| (r.key.as_str(), r)).collect();
        let mut frames = Vec::new();
        let mut kept = Vec::new();
        for mut subscription in std::mem::take(&mut self.subscriptions) {
            let advance = subscription.advance(head, &changes, &current, |record| {
                readable_by(&self.row, record, &self.viewer)
            });
            match advance {
                Advance::Nothing => kept.push(subscription),
                Advance::Resync => kept.push(self.resync(subscription, &mut frames).await),
                Advance::Changes { records, removed } => {
                    frames.extend(changes_chunks(&subscription.id, &records, &removed));
                    kept.push(subscription);
                }
            }
        }
        self.subscriptions = kept;
        Ok(frames)
    }

    /// Check the reader still reads the Artifact, then send what changed. New
    /// record rules send every page a fresh snapshot, since they can change
    /// which records it may read.
    async fn recheck(&mut self) -> Result<Vec<Value>, ArtifactError> {
        let (_, row) = page_reader(&self.tasks, &self.artifacts, &self.token).await?;
        let rules_changed = row.record_rules != self.row.record_rules;
        self.row = row;
        if !rules_changed {
            return Ok(self.pull().await);
        }
        let mut frames = Vec::new();
        let mut kept = Vec::new();
        for subscription in std::mem::take(&mut self.subscriptions) {
            kept.push(self.resync(subscription, &mut frames).await);
        }
        self.subscriptions = kept;
        Ok(frames)
    }

    /// Send `subscription` a fresh snapshot. A failed read is logged and leaves
    /// its cursor where it was, so the next pull resyncs it again; a snapshot
    /// reads only the database, so no failure here means the reader lost access.
    async fn resync(
        &self,
        mut subscription: Subscription,
        frames: &mut Vec<Value>,
    ) -> Subscription {
        match self.snapshot(&mut subscription).await {
            Ok(snapshot) => {
                subscription.stale = false;
                frames.extend(snapshot);
            }
            Err(error) => {
                subscription.stale = true;
                warn!(error = %moatless_core::error::causal_chain(&error), artifact_id = %self.row.artifact_id, "Failed to resync an artifact record subscription");
            }
        }
        subscription
    }

    /// The `Exit` that ends each subscription when the socket closes on `error`.
    fn ended(&self, error: &ArtifactError) -> Vec<Value> {
        self.subscriptions
            .iter()
            .map(|subscription| failed(&subscription.id, error))
            .collect()
    }
}

/// What the socket does after one event: send frames and carry on, send them
/// and close, or answer a WebSocket ping.
enum Inbound {
    Frames(Vec<Value>),
    End(Vec<Value>),
    Pong(axum::body::Bytes),
}

/// Write what `inbound` sends, answering whether the socket stays open.
async fn send(socket: &mut WebSocket, inbound: Inbound) -> bool {
    let (frames, open) = match inbound {
        Inbound::Frames(frames) => (frames, true),
        Inbound::End(frames) => (frames, false),
        Inbound::Pong(payload) => return socket.send(Message::Pong(payload)).await.is_ok(),
    };
    for frame in frames {
        if socket
            .send(Message::Text(frame.to_string().into()))
            .await
            .is_err()
        {
            return false;
        }
    }
    if !open {
        // The socket closes either way; a failed close frame changes nothing.
        let _ = socket.send(Message::Close(None)).await;
    }
    open
}

/// Whether `error` means the reader may no longer read the Artifact's records,
/// rather than a failure the next read may not repeat.
fn ends_access(error: &ArtifactError) -> bool {
    matches!(
        error,
        ArtifactError::Unauthorized
            | ArtifactError::NotFound(_)
            | ArtifactError::TaskNotFound(_)
            | ArtifactError::Forbidden(_)
    )
}

fn until_unix(exp: u64) -> Duration {
    let now = u64::try_from(chrono::Utc::now().timestamp()).unwrap_or_default();
    Duration::from_secs(exp.saturating_sub(now))
}

fn chunk(id: &Value, value: Value) -> Value {
    json!({ "_tag": "Chunk", "requestId": id, "values": [value] })
}

fn snapshot_chunk(id: &Value, records: &[&ArtifactRecord], done: bool) -> Value {
    let items: Vec<_> = records.iter().map(|record| to_response(record)).collect();
    chunk(
        id,
        json!({ "_tag": "Snapshot", "items": items, "done": done }),
    )
}

/// `Changes` chunks for `records` and `removed`; the removals ride on the
/// first.
fn changes_chunks(id: &Value, records: &[ArtifactRecord], removed: &[String]) -> Vec<Value> {
    let mut batches: Vec<&[ArtifactRecord]> = records.chunks(RECORDS_PER_CHUNK).collect();
    if batches.is_empty() {
        batches.push(&[]);
    }
    batches
        .iter()
        .enumerate()
        .map(|(i, batch)| {
            let items: Vec<_> = batch.iter().map(to_response).collect();
            let removed: &[String] = if i == 0 { removed } else { &[] };
            chunk(
                id,
                json!({ "_tag": "Changes", "items": items, "removed": removed }),
            )
        })
        .collect()
}

fn exit(id: &Value, cause: Value) -> Value {
    json!({
        "_tag": "Exit",
        "requestId": id,
        "exit": { "_tag": "Failure", "cause": [cause] },
    })
}

fn die(id: &Value, defect: String) -> Value {
    exit(id, json!({ "_tag": "Die", "defect": defect }))
}

/// The `Exit` a subscription ends with when the socket can no longer serve
/// it; `code` is the one the page script's HTTP calls reject with.
fn failed(id: &Value, error: &ArtifactError) -> Value {
    let code = match error {
        ArtifactError::Unauthorized => "unauthenticated",
        ArtifactError::NotFound(_) | ArtifactError::TaskNotFound(_) => "not-found",
        ArtifactError::Forbidden(_) | ArtifactError::PermissionDenied { .. } => "permission-denied",
        _ => "failed",
    };
    exit(
        id,
        json!({
            "_tag": "Fail",
            "error": { "_tag": "ArtifactRecordError", "code": code, "message": error.to_string() },
        }),
    )
}

fn defect(message: String) -> Value {
    json!({ "_tag": "Defect", "defect": message })
}

#[cfg(test)]
#[path = "record_socket_tests.rs"]
mod tests;
