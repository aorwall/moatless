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
    }
    Ok(())
}

/// Stream real-time events from the SSE endpoint, filtered by task_id.
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
                |v| {
                    let s = serde_json::to_string(v).unwrap_or_default();
                    if s.len() > 80 {
                        format!("{}...", &s[..77])
                    } else {
                        s
                    }
                },
            );
            vec![timestamp, event_type, sub_type, error_str, payload]
        })
        .collect();

    output::print_table(headers, &rows);
}
