//! Tails sub-agent transcripts and threads them onto the spawning tool call.
//!
//! The TUI writes each sub-agent's conversation to its own file under
//! `<project_dir>/<session_id>/subagents/agent-<agent_id>.jsonl`, outside the
//! main transcript the tailer follows. Without this watcher those records never
//! become `SdkMessage`s, so a `Task` call shows as a tool result with no
//! visible sub-conversation — unlike the subprocess transport, where the CLI
//! interleaves them into the one stream.
//!
//! Records are buffered until the spawning `tool_use_id` is known. That id only
//! reaches the main transcript when the sub-agent finishes, and consumers key
//! sub-agent messages off it, so emitting earlier would produce messages that
//! thread onto nothing.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, BufReader};

use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::error::Result;
use crate::types::content::Usage;
use crate::types::messages::SdkMessage;

use super::translator::{translate_record, Translated, TurnAccumulator};

/// Maps sub-agent id → the `tool_use_id` of the `Task` call that spawned it,
/// as learned from the main transcript.
pub(crate) type SubagentRegistry = Arc<Mutex<HashMap<String, String>>>;

const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Directory the TUI writes sub-agent transcripts to for a session.
pub(crate) fn subagent_dir(project_dir: &Path, session_id: &str) -> PathBuf {
    project_dir.join(session_id).join("subagents")
}

fn agent_id_from_path(path: &Path) -> Option<String> {
    path.file_stem()?
        .to_str()?
        .strip_prefix("agent-")
        .map(str::to_string)
}

/// Poll the sub-agent directory and tail each transcript that appears.
pub(crate) fn spawn_subagent_watcher(
    dir: PathBuf,
    registry: SubagentRegistry,
    tx: mpsc::Sender<Result<SdkMessage>>,
    cancellation_token: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tailing: HashSet<PathBuf> = HashSet::new();
        loop {
            if cancellation_token.is_cancelled() {
                return;
            }

            if let Ok(mut entries) = tokio::fs::read_dir(&dir).await {
                while let Ok(Some(entry)) = entries.next_entry().await {
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) != Some("jsonl")
                        || tailing.contains(&path)
                    {
                        continue;
                    }
                    let Some(agent_id) = agent_id_from_path(&path) else {
                        continue;
                    };
                    tailing.insert(path.clone());
                    debug!(agent_id = %agent_id, path = %path.display(), "Tailing sub-agent transcript");
                    let task_tx = tx.clone();
                    let task_registry = Arc::clone(&registry);
                    let task_token = cancellation_token.clone();
                    tokio::spawn(async move {
                        subagent_tail_loop(path, agent_id, task_registry, task_tx, task_token)
                            .await;
                    });
                }
            }

            tokio::select! {
                () = cancellation_token.cancelled() => return,
                () = tokio::time::sleep(POLL_INTERVAL) => {}
            }
        }
    })
}

/// Total a finished sub-agent's transcript.
///
/// `toolUseResult` on the main stream reports only the sub-agent's final API
/// call — its `usage`, `iterations`, and `totalTokens` all omit the earlier
/// messages — so the transcript is the only complete source. Safe to read
/// directly: the file is fully written by the time the spawning `Task` call
/// returns.
///
/// Deduped by `message.id` for the same reason the turn accumulator is: the
/// TUI writes one record per content block, each repeating the message's usage.
pub(crate) async fn sum_transcript_usage(path: &Path) -> Option<Usage> {
    // Streamed line-by-line: a transcript carries full tool outputs, so reading
    // it whole would spike with the sub-agent's output size.
    let file = tokio::fs::File::open(path).await.ok()?;
    let mut lines = BufReader::new(file).lines();
    let mut total = Usage {
        input_tokens: 0,
        output_tokens: 0,
        cache_read_input_tokens: Some(0),
        cache_creation_input_tokens: Some(0),
        service_tier: None,
        cache_creation: None,
        server_tool_use: None,
    };
    let mut seen: HashSet<String> = HashSet::new();
    let mut found = false;

    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if value.get("type").and_then(|t| t.as_str()) != Some("assistant") {
            continue;
        }
        let Some(message) = value.get("message") else {
            continue;
        };
        if let Some(id) = message.get("id").and_then(|i| i.as_str()) {
            if !seen.insert(id.to_string()) {
                continue;
            }
        }
        let Some(usage) = message
            .get("usage")
            .and_then(|u| serde_json::from_value::<Usage>(u.clone()).ok())
        else {
            continue;
        };
        found = true;
        total.input_tokens += usage.input_tokens;
        total.output_tokens += usage.output_tokens;
        total.cache_read_input_tokens = Some(
            total.cache_read_input_tokens.unwrap_or(0) + usage.cache_read_input_tokens.unwrap_or(0),
        );
        total.cache_creation_input_tokens = Some(
            total.cache_creation_input_tokens.unwrap_or(0)
                + usage.cache_creation_input_tokens.unwrap_or(0),
        );
    }

    found.then_some(total)
}

/// Attach the spawning tool call to a sub-agent message so consumers can thread
/// it. This is the same field the subprocess transport populates.
fn set_parent(msg: &mut SdkMessage, tool_use_id: &str) {
    let parent = Some(tool_use_id.to_string());
    match msg {
        SdkMessage::Assistant(m) => m.parent_tool_use_id = parent,
        SdkMessage::User(m) => m.parent_tool_use_id = parent,
        SdkMessage::UserReplay(m) => m.parent_tool_use_id = parent,
        _ => {}
    }
}

/// Wait for the spawning `tool_use_id`, then stream the sub-agent transcript.
///
/// Nothing can be emitted before the `tool_use_id` is known (consumers key
/// sub-agent messages off it, and it only reaches the main transcript when the
/// sub-agent finishes), so this waits rather than buffering. That keeps memory
/// flat per sub-agent no matter how much it produced — a waiting task holds two
/// short strings, not its conversation. Registration also means the file is
/// complete, so it is read once to EOF instead of tailed.
///
/// `Result` records are dropped: a sub-agent's turn boundary is not the run's,
/// and forwarding one would end the run early.
async fn subagent_tail_loop(
    path: PathBuf,
    agent_id: String,
    registry: SubagentRegistry,
    tx: mpsc::Sender<Result<SdkMessage>>,
    cancellation_token: CancellationToken,
) {
    let tool_use_id = loop {
        if cancellation_token.is_cancelled() {
            return;
        }
        // Taken, not cloned: each agent id is consumed once, so the registry
        // doesn't grow across a long session.
        if let Some(id) = registry.lock().await.remove(&agent_id) {
            break id;
        }
        tokio::select! {
            () = cancellation_token.cancelled() => return,
            () = tokio::time::sleep(POLL_INTERVAL) => {}
        }
    };

    let file = match tokio::fs::File::open(&path).await {
        Ok(f) => f,
        Err(e) => {
            warn!(error = %e, path = %path.display(), "Failed to open sub-agent transcript");
            return;
        }
    };

    // Scratch accumulator: reuses the main translation path (MCP tool-result
    // normalization, content shapes) without touching the run's turn totals,
    // which are summed from this transcript on the main stream instead.
    let mut acc = TurnAccumulator::new(agent_id.clone());
    let mut reader = BufReader::new(file);
    let mut buf = String::new();
    let mut sent = 0usize;

    loop {
        if cancellation_token.is_cancelled() {
            return;
        }

        buf.clear();
        let n = tokio::select! {
            () = cancellation_token.cancelled() => return,
            res = reader.read_line(&mut buf) => match res {
                Ok(n) => n,
                Err(e) => {
                    warn!(error = %e, agent_id = %agent_id, "Sub-agent transcript read error");
                    return;
                }
            }
        };
        if n == 0 {
            debug!(
                agent_id = %agent_id,
                tool_use_id = %tool_use_id,
                count = sent,
                "Sub-agent transcript forwarded"
            );
            return;
        }

        let line = buf.trim_end_matches(['\n', '\r']);
        if line.is_empty() {
            continue;
        }
        let value: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                warn!(error = %e, agent_id = %agent_id, "Skipping malformed sub-agent line");
                continue;
            }
        };

        if let Translated::One(mut msg) = translate_record(value, &mut acc) {
            if matches!(*msg, SdkMessage::Result(_)) {
                continue;
            }
            set_parent(&mut msg, &tool_use_id);
            if tx.send(Ok(*msg)).await.is_err() {
                debug!(agent_id = %agent_id, "Consumer dropped — sub-agent tailer exiting");
                return;
            }
            sent += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_id_is_parsed_from_the_transcript_filename() {
        let p = PathBuf::from("/x/sess/subagents/agent-afaa9b2d9aab65250.jsonl");
        assert_eq!(
            agent_id_from_path(&p).as_deref(),
            Some("afaa9b2d9aab65250")
        );
    }

    #[test]
    fn non_agent_files_are_ignored() {
        assert_eq!(agent_id_from_path(&PathBuf::from("/x/notes.jsonl")), None);
    }

    #[test]
    fn subagent_dir_matches_the_tui_layout() {
        let dir = subagent_dir(Path::new("/p/-opt-moatless-workspace"), "sess-1");
        assert_eq!(
            dir,
            PathBuf::from("/p/-opt-moatless-workspace/sess-1/subagents")
        );
    }

    /// `toolUseResult` reports only the sub-agent's last call, so the total has
    /// to come from every message in the transcript — deduped by `message.id`,
    /// since the TUI repeats usage on each content-block record.
    #[tokio::test]
    async fn transcript_usage_sums_all_messages_once_each() {
        let dir = std::env::temp_dir().join("sdk-subagent-usage-test");
        tokio::fs::create_dir_all(&dir).await.expect("mkdir");
        let path = dir.join("agent-abc.jsonl");

        let rec = |id: &str, out: u64, block: &str| {
            serde_json::json!({
                "type": "assistant",
                "uuid": format!("{id}-{block}"),
                "message": {
                    "id": id,
                    "role": "assistant",
                    "content": [{"type": block, "text": "x"}],
                    "usage": {"input_tokens": 2, "output_tokens": out}
                }
            })
            .to_string()
        };
        let lines = [
            rec("msg_a", 97, "thinking"),
            rec("msg_a", 97, "text"), // same message, second block
            rec("msg_b", 121, "text"),
            rec("msg_c", 31, "text"),
        ]
        .join("\n");
        tokio::fs::write(&path, lines).await.expect("write");

        let usage = sum_transcript_usage(&path).await.expect("usage");
        assert_eq!(usage.output_tokens, 97 + 121 + 31);
        assert_eq!(usage.input_tokens, 2 * 3);

        tokio::fs::remove_file(&path).await.ok();
    }

    #[tokio::test]
    async fn transcript_usage_is_none_when_there_is_nothing_to_count() {
        let missing = PathBuf::from("/nonexistent/agent-x.jsonl");
        assert!(sum_transcript_usage(&missing).await.is_none());
    }

    #[test]
    fn set_parent_threads_assistant_and_user_messages() {
        use crate::types::content::ApiMessage;

        let api = || ApiMessage {
            id: None,
            role: "assistant".to_string(),
            model: None,
            content: vec![],
            stop_reason: None,
            stop_sequence: None,
            usage: None,
        };

        let mut assistant = SdkMessage::Assistant(crate::types::messages::AssistantMessage {
            uuid: "u1".to_string(),
            session_id: "s".to_string(),
            message: api(),
            parent_tool_use_id: None,
            error: None,
            is_api_error_message: None,
            api_error: None,
            request_id: None,
        });
        set_parent(&mut assistant, "toolu_x");
        match assistant {
            SdkMessage::Assistant(m) => {
                assert_eq!(m.parent_tool_use_id.as_deref(), Some("toolu_x"));
            }
            _ => panic!("expected assistant"),
        }
    }
}
