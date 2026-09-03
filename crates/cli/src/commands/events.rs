//! Events commands: list task lifecycle events.

use std::fmt::Write;

use clap::Subcommand;
use futures_util::StreamExt;

use crate::client::{ClientError, MoatClient};
use crate::output;

#[derive(Subcommand)]
pub enum EventAction {
    /// List events for a task
    List {
        task_id: String,
        /// Comma-separated event type filter (e.g. sandbox,server,agent)
        #[arg(long)]
        types: Option<String>,
        /// Maximum number of events to return
        #[arg(long, default_value = "50")]
        limit: i64,
        /// Show only error events
        #[arg(long)]
        errors: bool,
        /// Follow events in real-time (stream via SSE)
        #[arg(long, short = 'f')]
        follow: bool,
    },
    /// Stream live events as newline-delimited JSON, one object per line.
    ///
    /// Unlike `list --follow`, this prints no history and no decoration, so it
    /// can be piped into anything that reads NDJSON. Runs until interrupted.
    ///
    /// Example:
    ///   moat events stream --task abc123 --types sandbox,agent
    Stream {
        /// Only events for this task. Omit for every task the caller can see.
        #[arg(long)]
        task: Option<String>,
        /// Comma-separated event type filter (e.g. sandbox,server,agent)
        #[arg(long)]
        types: Option<String>,
        /// Include the stream's keep-alive pings, which are filtered by default.
        #[arg(long)]
        include_pings: bool,
    },
}

pub async fn run(client: &MoatClient, action: EventAction, json: bool) -> Result<(), ClientError> {
    match action {
        EventAction::List {
            task_id,
            types,
            limit,
            errors,
            follow,
        } => {
            // Fetch and display history first.
            let mut path = format!("/api/v1/tasks/{task_id}/events/history?limit={limit}");
            if let Some(ref t) = types {
                let _ = write!(path, "&types={t}");
            }

            let resp = client.raw_get(&path).await?;
            let items = resp
                .get("items")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();

            let items: Vec<serde_json::Value> = if errors {
                items
                    .into_iter()
                    .filter(|e| {
                        e.get("isError")
                            .and_then(serde_json::Value::as_bool)
                            .unwrap_or(false)
                    })
                    .collect()
            } else {
                items
            };

            if json {
                let pretty = serde_json::to_string_pretty(&items).unwrap_or_default();
                println!("{pretty}");
            } else {
                format_events(&items);
            }

            // If --follow, stream real-time events via SSE.
            if follow {
                follow_events(client, &task_id, types.as_deref(), errors, json).await?;
            }
        }
        EventAction::Stream {
            task,
            types,
            include_pings,
        } => {
            stream_events(client, task.as_deref(), types.as_deref(), include_pings).await?;
        }
    }
    Ok(())
}

/// Stream events as NDJSON — one compact JSON object per line, nothing else on
/// stdout, so the output is consumable by a program rather than a reader.
async fn stream_events(
    client: &MoatClient,
    task_id: Option<&str>,
    types: Option<&str>,
    include_pings: bool,
) -> Result<(), ClientError> {
    let mut path = "/api/v1/events/stream".to_string();
    if let Some(t) = types {
        let _ = write!(path, "?types={t}");
    }

    let resp = client.raw_get_stream(&path).await?;
    let mut stream = resp.bytes_stream();
    let mut buf = String::new();

    loop {
        tokio::select! {
            chunk = stream.next() => {
                match chunk {
                    Some(Ok(bytes)) => {
                        buf.push_str(&String::from_utf8_lossy(&bytes));
                        while let Some(pos) = buf.find('\n') {
                            let line: String = buf.drain(..=pos).collect();
                            let line = line.trim();
                            if line.is_empty() {
                                continue;
                            }
                            let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else {
                                continue;
                            };
                            if !include_pings
                                && event.get("type").and_then(|v| v.as_str()) == Some("ping")
                            {
                                continue;
                            }
                            if let Some(wanted) = task_id {
                                // The stream is global; a task filter is applied here.
                                if event.get("taskId").and_then(|v| v.as_str()) != Some(wanted) {
                                    continue;
                                }
                            }
                            println!("{}", serde_json::to_string(&event).unwrap_or_default());
                        }
                    }
                    Some(Err(e)) => {
                        eprintln!("Stream error: {e}");
                        break;
                    }
                    None => break,
                }
            }
            _ = tokio::signal::ctrl_c() => break,
        }
    }
    Ok(())
}

/// Stream real-time events from the SSE endpoint, filtered by `task_id`.
async fn follow_events(
    client: &MoatClient,
    task_id: &str,
    types: Option<&str>,
    errors_only: bool,
    json: bool,
) -> Result<(), ClientError> {
    let mut path = "/api/v1/events/stream".to_string();
    if let Some(t) = types {
        let _ = write!(path, "?types={t}");
    }

    let resp = client.raw_get_stream(&path).await?;
    let mut stream = resp.bytes_stream();
    let mut buf = String::new();

    eprintln!("Following events (Ctrl+C to stop)...");

    loop {
        tokio::select! {
            chunk = stream.next() => {
                match chunk {
                    Some(Ok(bytes)) => {
                        buf.push_str(&String::from_utf8_lossy(&bytes));
                        // Process complete NDJSON lines.
                        while let Some(pos) = buf.find('\n') {
                            let line: String = buf.drain(..=pos).collect();
                            let line = line.trim();
                            if line.is_empty() {
                                continue;
                            }
                            if let Ok(event) = serde_json::from_str::<serde_json::Value>(line) {
                                // Skip ping events.
                                if event.get("type").and_then(|v| v.as_str()) == Some("ping") {
                                    continue;
                                }
                                // Filter by task_id (stream is global).
                                let event_task = event.get("taskId").and_then(|v| v.as_str());
                                if event_task != Some(task_id) {
                                    continue;
                                }
                                // Filter errors if requested.
                                if errors_only {
                                    let is_err = event.get("isError")
                                        .and_then(serde_json::Value::as_bool)
                                        .unwrap_or(false);
                                    if !is_err {
                                        continue;
                                    }
                                }
                                if json {
                                    let pretty = serde_json::to_string_pretty(&event).unwrap_or_default();
                                    println!("{pretty}");
                                } else {
                                    format_events(&[event]);
                                }
                            }
                        }
                    }
                    Some(Err(e)) => {
                        eprintln!("Stream error: {e}");
                        break;
                    }
                    None => break,
                }
            }
            _ = tokio::signal::ctrl_c() => {
                eprintln!("\nStopped.");
                break;
            }
        }
    }
    Ok(())
}

fn format_events(items: &[serde_json::Value]) {
    let headers = &["TIMESTAMP", "TYPE", "SUB_TYPE", "ERROR", "PAYLOAD"];
    let rows: Vec<Vec<String>> = items
        .iter()
        .map(|e| {
            let timestamp = e
                .get("timestamp")
                .and_then(|v| v.as_str())
                .unwrap_or("-")
                .to_string();
            // Truncate timestamp to seconds for readability
            let timestamp = if timestamp.len() > 19 {
                timestamp[..19].to_string()
            } else {
                timestamp
            };
            let event_type = e
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("-")
                .to_string();
            let sub_type = e
                .get("subType")
                .and_then(|v| v.as_str())
                .unwrap_or("-")
                .to_string();
            let is_error = e
                .get("isError")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let error_str = if is_error { "YES" } else { "-" }.to_string();
            let payload = e.get("payload").map_or_else(
                || "{}".to_string(),
                |v| truncate_payload(&serde_json::to_string(v).unwrap_or_default()),
            );
            vec![timestamp, event_type, sub_type, error_str, payload]
        })
        .collect();

    output::print_table(headers, &rows);
}

/// Truncate a payload string to ~80 chars for table display, appending `...`.
///
/// Truncates on a UTF-8 char boundary — naive byte slicing panics when the cut
/// index lands inside a multi-byte character (e.g. an em-dash straddling byte 77).
fn truncate_payload(s: &str) -> String {
    if s.len() <= 80 {
        return s.to_string();
    }
    let cut = s
        .char_indices()
        .take_while(|(i, _)| *i <= 77)
        .last()
        .map_or(0, |(i, _)| i);
    format!("{}...", &s[..cut])
}

#[cfg(test)]
mod tests {
    use super::truncate_payload;

    #[test]
    fn short_payload_is_unchanged() {
        let s = r#"{"message":"Starting"}"#;
        assert_eq!(truncate_payload(s), s);
    }

    #[test]
    fn long_ascii_payload_is_truncated_with_ellipsis() {
        let s = "a".repeat(200);
        let out = truncate_payload(&s);
        assert!(out.ends_with("..."));
        assert!(out.len() <= 81);
    }

    #[test]
    fn does_not_panic_on_multibyte_char_at_boundary() {
        // An em-dash (3 bytes) positioned so the 77-byte cut lands mid-character.
        // The pre-fix code sliced at byte 77 and panicked here.
        let s = format!("{}—{}", "x".repeat(75), "y".repeat(50));
        let out = truncate_payload(&s);
        assert!(out.ends_with("..."));
        // Cut must land on a boundary — the em-dash is kept or dropped whole.
        assert!(out.is_char_boundary(out.len() - 3));
    }
}
