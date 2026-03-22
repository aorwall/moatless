//! Reusable SSE/NDJSON event collector for integration tests.
//!
//! Spawns a background task that reads the global event stream and accumulates
//! parsed events.  Tests can query events by task_id, type, and sub_type.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::StreamExt;
use tokio::task::JoinHandle;

use crate::client::MoatClient;

/// A single parsed NDJSON event from the global stream.
#[derive(Clone, Debug)]
pub struct SseEvent {
    pub event_type: String,
    pub sub_type: Option<String>,
    pub task_id: Option<String>,
    pub is_error: bool,
    pub payload: serde_json::Value,
    pub raw: serde_json::Value,
}

/// Collects events from the global NDJSON stream in the background.
pub struct EventCollector {
    events: Arc<Mutex<Vec<SseEvent>>>,
    handle: JoinHandle<()>,
}

impl EventCollector {
    /// Start collecting events.  Spawns a tokio task that reads from
    /// `/api/v1/events/stream` and accumulates parsed events.
    pub async fn start(client: &MoatClient) -> Result<Self, crate::client::ClientError> {
        let resp = client.raw_get_stream("/api/v1/events/stream").await?;
        let events: Arc<Mutex<Vec<SseEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let events_clone = Arc::clone(&events);

        let handle = tokio::spawn(async move {
            let mut stream = resp.bytes_stream();
            let mut buf = String::new();

            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(bytes) => {
                        buf.push_str(&String::from_utf8_lossy(&bytes));
                        while let Some(pos) = buf.find('\n') {
                            let line: String = buf.drain(..=pos).collect();
                            let line = line.trim();
                            if line.is_empty() {
                                continue;
                            }
                            if let Ok(event) = serde_json::from_str::<serde_json::Value>(line) {
                                // Skip ping/keepalive events.
                                if event.get("type").and_then(|v| v.as_str()) == Some("ping") {
                                    continue;
                                }

                                let parsed = SseEvent {
                                    event_type: event
                                        .get("type")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string(),
                                    sub_type: event
                                        .get("subType")
                                        .and_then(|v| v.as_str())
                                        .map(ToString::to_string),
                                    task_id: event
                                        .get("taskId")
                                        .and_then(|v| v.as_str())
                                        .map(ToString::to_string),
                                    is_error: event
                                        .get("isError")
                                        .and_then(serde_json::Value::as_bool)
                                        .unwrap_or(false),
                                    payload: event
                                        .get("payload")
                                        .cloned()
                                        .unwrap_or(serde_json::Value::Null),
                                    raw: event,
                                };

                                if let Ok(mut vec) = events_clone.lock() {
                                    vec.push(parsed);
                                }
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Self { events, handle })
    }

    /// Stop the background collector by aborting its task.
    pub fn stop(self) {
        self.handle.abort();
    }

    /// Return all events matching a given task_id.
    pub fn events_for_task(&self, task_id: &str) -> Vec<SseEvent> {
        self.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .filter(|e| e.task_id.as_deref() == Some(task_id))
            .cloned()
            .collect()
    }

    /// Wait for an event matching `event_type` (and optionally `sub_type`) for
    /// the given `task_id`.  Polls at 500ms intervals up to `timeout`.
    pub async fn wait_for_event(
        &self,
        event_type: &str,
        sub_type: Option<&str>,
        task_id: &str,
        timeout: Duration,
    ) -> Option<SseEvent> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            {
                let guard = self.events.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                if let Some(evt) = guard.iter().rev().find(|e| {
                    e.event_type == event_type
                        && e.task_id.as_deref() == Some(task_id)
                        && match sub_type {
                            Some(st) => e.sub_type.as_deref() == Some(st),
                            None => true,
                        }
                }) {
                    return Some(evt.clone());
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// Wait for the Nth occurrence (1-based) of an event matching `event_type`
    /// and `sub_type` for the given `task_id`.
    pub async fn wait_for_nth_event(
        &self,
        event_type: &str,
        sub_type: &str,
        task_id: &str,
        n: usize,
        timeout: Duration,
    ) -> Option<SseEvent> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            {
                let guard = self.events.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                let matches: Vec<_> = guard
                    .iter()
                    .filter(|e| {
                        e.event_type == event_type
                            && e.sub_type.as_deref() == Some(sub_type)
                            && e.task_id.as_deref() == Some(task_id)
                    })
                    .collect();
                if matches.len() >= n {
                    return Some(matches[n - 1].clone());
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// Count events matching `event_type` and `sub_type` for a task.
    pub fn count_events(&self, event_type: &str, sub_type: &str, task_id: &str) -> usize {
        self.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .filter(|e| {
                e.event_type == event_type
                    && e.sub_type.as_deref() == Some(sub_type)
                    && e.task_id.as_deref() == Some(task_id)
            })
            .count()
    }

    /// Wait for an event matching type/subType where a payload field equals a value.
    ///
    /// Useful for filtering events by payload content, e.g. waiting for a
    /// `sandbox.initializing` event with `payload.reason == "PluginReposCloned"`.
    pub async fn wait_for_event_with_payload(
        &self,
        event_type: &str,
        sub_type: Option<&str>,
        task_id: &str,
        payload_key: &str,
        payload_value: &str,
        timeout: Duration,
    ) -> Option<SseEvent> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            {
                let guard = self.events.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                if let Some(evt) = guard.iter().rev().find(|e| {
                    e.event_type == event_type
                        && e.task_id.as_deref() == Some(task_id)
                        && match sub_type {
                            Some(st) => e.sub_type.as_deref() == Some(st),
                            None => true,
                        }
                        && e.payload
                            .get(payload_key)
                            .and_then(|v| v.as_str())
                            == Some(payload_value)
                }) {
                    return Some(evt.clone());
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// Return a summary of event counts by type.subType for a given task.
    ///
    /// Useful for test diagnostics.
    pub fn event_summary(&self, task_id: &str) -> Vec<(String, usize)> {
        let guard = self.events.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for e in guard.iter().filter(|e| e.task_id.as_deref() == Some(task_id)) {
            let key = match &e.sub_type {
                Some(st) => format!("{}.{}", e.event_type, st),
                None => e.event_type.clone(),
            };
            *counts.entry(key).or_default() += 1;
        }
        let mut result: Vec<_> = counts.into_iter().collect();
        result.sort_by(|a, b| a.0.cmp(&b.0));
        result
    }
}
