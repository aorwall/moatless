//! Hook-driven lifecycle plumbing for the tmux Claude TUI.
//!
//! Registers `SessionStart` and `Stop` hooks via inline `--settings` JSON; a
//! relay shell script writes each event payload (`"<event>\t<payload>\n"`) to a
//! FIFO this module owns, which the supervisor drains asynchronously.
//!
//! Caller owns the [`HookHarness`]; `Drop` removes the per-run temp dir, FIFO,
//! and script.

// reason: items are introduced module-by-module across the rollout (see
// "Implementation order" in the plan). Each step wires another helper in.
#![allow(dead_code)]

use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::error::{Result, SdkError};

/// Per-run hook plumbing: temp dir, FIFO, relay script, and the inline
/// `--settings` JSON to pass to `claude`.
#[derive(Debug)]
pub(crate) struct HookHarness {
    pub(crate) tmp_dir: PathBuf,
    pub(crate) fifo_path: PathBuf,
    pub(crate) script_path: PathBuf,
    pub(crate) settings_json: String,
}

impl HookHarness {
    /// Build a fresh harness: create the temp dir, mkfifo, write the relay
    /// script, and assemble the inline settings JSON.
    pub(crate) fn create() -> Result<Self> {
        let tmp_root =
            std::env::var_os("TMPDIR").map_or_else(|| PathBuf::from("/tmp"), PathBuf::from);

        let pid = std::process::id();
        let uuid = uuid::Uuid::new_v4().simple().to_string();
        let tmp_dir = tmp_root.join(format!("claude-sdk-tmux-{pid}-{uuid}"));

        std::fs::create_dir_all(&tmp_dir).map_err(|e| {
            SdkError::HookHarnessFailed(format!("create tmp dir {}: {e}", tmp_dir.display()))
        })?;

        let fifo_path = tmp_dir.join("events.fifo");
        let script_path = tmp_dir.join("hook.sh");

        mkfifo(&fifo_path, 0o600_u32)?;

        std::fs::write(&script_path, SCRIPT_BODY).map_err(|e| {
            SdkError::HookHarnessFailed(format!(
                "write relay script {}: {e}",
                script_path.display()
            ))
        })?;
        // Make script executable.
        let mut perms = std::fs::metadata(&script_path)
            .map_err(|e| {
                SdkError::HookHarnessFailed(format!(
                    "stat relay script {}: {e}",
                    script_path.display()
                ))
            })?
            .permissions();
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o700);
        std::fs::set_permissions(&script_path, perms).map_err(|e| {
            SdkError::HookHarnessFailed(format!(
                "chmod relay script {}: {e}",
                script_path.display()
            ))
        })?;

        let settings_json = build_settings_json(&script_path)?;

        Ok(Self {
            tmp_dir,
            fifo_path,
            script_path,
            settings_json,
        })
    }
}

impl Drop for HookHarness {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.fifo_path);
        let _ = std::fs::remove_file(&self.script_path);
        let _ = std::fs::remove_dir(&self.tmp_dir);
    }
}

/// Relay script body. Writes one line `"<event>\t<payload>\n"` to
/// `$CLAUDE_P_FIFO` per invocation. Single quotes in `printf` guard against `%`
/// interpolation in the payload.
const SCRIPT_BODY: &str = r#"#!/bin/sh
# Relay a Claude Code hook event to the claude-agent-sdk hook FIFO.
#   $1     = event name (e.g. "Stop", "SessionStart")
#   stdin  = hook JSON payload (single line, no embedded newlines).
set -eu
event="$1"
fifo="${CLAUDE_P_FIFO:?missing CLAUDE_P_FIFO}"
payload="$(cat)"
printf '%s\t%s\n' "$event" "$payload" >> "$fifo"
exit 0
"#;

/// Build the inline `--settings` JSON registering `SessionStart` + Stop hooks.
fn build_settings_json(script_path: &Path) -> Result<String> {
    let script = script_path.to_string_lossy().into_owned();
    let entry = |event: &str| {
        serde_json::json!({
            "matcher": "*",
            "hooks": [{
                "type": "command",
                "command": format!("{script} {event}"),
            }],
        })
    };
    let settings = serde_json::json!({
        "hooks": {
            "SessionStart": [entry("SessionStart")],
            "Stop": [entry("Stop")],
        },
    });
    serde_json::to_string(&settings)
        .map_err(|e| SdkError::HookHarnessFailed(format!("serialize settings JSON: {e}")))
}

/// Hook events we recognize. Anything else is `Unknown`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HookEvent {
    SessionStart,
    Stop,
    Unknown,
}

impl HookEvent {
    fn from_str(s: &str) -> Self {
        match s {
            "SessionStart" => Self::SessionStart,
            "Stop" => Self::Stop,
            _ => Self::Unknown,
        }
    }
}

/// A parsed line from the FIFO: `"<event>\t<payload>"`.
#[derive(Debug, Clone)]
pub(crate) struct HookLine {
    pub(crate) event: HookEvent,
    pub(crate) payload: String,
}

/// Parse a single line emitted by the relay script. Tolerates trailing CR/LF.
pub(crate) fn parse_line(raw: &str) -> Option<HookLine> {
    let line = raw.trim_end_matches(['\n', '\r']);
    let tab = line.find('\t')?;
    Some(HookLine {
        event: HookEvent::from_str(&line[..tab]),
        payload: line[tab + 1..].to_string(),
    })
}

/// Pull `transcript_path` out of a Stop-hook payload.
pub(crate) fn extract_transcript_path(payload_json: &str) -> Option<String> {
    extract_string_field(payload_json, "transcript_path")
}

/// Pull `last_assistant_message` out of a Stop-hook payload (recent Claude
/// versions include this string; older versions don't).
pub(crate) fn extract_last_assistant_message(payload_json: &str) -> Option<String> {
    extract_string_field(payload_json, "last_assistant_message")
}

/// Pull `session_id` out of a Stop/SessionStart hook payload.
pub(crate) fn extract_session_id(payload_json: &str) -> Option<String> {
    extract_string_field(payload_json, "session_id")
}

fn extract_string_field(payload_json: &str, field: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(payload_json).ok()?;
    v.as_object()?
        .get(field)?
        .as_str()
        .map(std::string::ToString::to_string)
}

/// Opens `O_RDONLY | O_RDWR` so reads block on data, not EOF, since the relay
/// writes one payload per connection; exits when `cancellation_token` fires.
/// MUST be called BEFORE spawning the child process, or its first hook write
/// blocks waiting for a reader.
pub(crate) fn spawn_reader(
    fifo_path: PathBuf,
    tx: mpsc::UnboundedSender<HookLine>,
    cancellation_token: CancellationToken,
) -> Result<tokio::task::JoinHandle<()>> {
    use std::io::{BufRead, BufReader};

    // Open O_RDWR | O_NONBLOCK on the FIFO. O_RDWR keeps a writer slot held
    // by us, so reads never see EOF from "all writers detached" — they just
    // park on the next byte. O_NONBLOCK gives us a fast open() and lets us
    // poll cancellation between reads.
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(&fifo_path)
        .map_err(SdkError::HookFifoFailed)?;

    let cancel_for_task = cancellation_token.clone();
    let handle = tokio::task::spawn_blocking(move || {
        let mut reader = BufReader::new(file);
        let mut line_buf = String::new();
        loop {
            if cancel_for_task.is_cancelled() {
                debug!("hook FIFO reader cancelled");
                return;
            }
            line_buf.clear();
            match reader.read_line(&mut line_buf) {
                Ok(0) => {
                    // FIFO closed (shouldn't happen with O_RDWR), or no data
                    // ready (depends on libc behavior). Sleep briefly and
                    // re-check cancellation.
                    std::thread::sleep(Duration::from_millis(25));
                }
                Ok(_n) => {
                    if let Some(line) = parse_line(&line_buf) {
                        if tx.send(line).is_err() {
                            debug!("hook FIFO reader: consumer dropped, exiting");
                            return;
                        }
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    // O_NONBLOCK: no data ready. Park briefly.
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    warn!(error = %e, "hook FIFO read error — reader exiting");
                    return;
                }
            }
        }
    });
    Ok(handle)
}

/// Convenience: drains a [`HookLine`] receiver and applies a callback to each
/// recognized event. Used by tests.
#[cfg(test)]
pub(crate) async fn for_each_line(
    rx: &mut mpsc::UnboundedReceiver<HookLine>,
    mut on_line: impl FnMut(HookLine),
) {
    while let Some(line) = rx.recv().await {
        on_line(line);
    }
}

/// Create a FIFO at `path` with the given mode (octal).
///
/// Shells out to the POSIX `mkfifo` binary to avoid an `unsafe` libc call;
/// required by POSIX and already assumed present by the rest of the tmux
/// backend.
fn mkfifo(path: &Path, mode: u32) -> Result<()> {
    let status = std::process::Command::new("mkfifo")
        .arg("-m")
        .arg(format!("{mode:o}"))
        .arg(path)
        .status()
        .map_err(SdkError::HookFifoFailed)?;
    if !status.success() {
        return Err(SdkError::HookHarnessFailed(format!(
            "mkfifo {} exited with {status:?}",
            path.display()
        )));
    }
    Ok(())
}

// reason: Arc is used by callers wanting cheap cloning of the harness across
// the supervisor + writer tasks.
#[allow(dead_code)]
pub(crate) type SharedHookHarness = Arc<HookHarness>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_json_has_session_start_and_stop_entries() {
        let script = PathBuf::from("/tmp/hook.sh");
        let json = build_settings_json(&script).expect("build settings");
        let v: serde_json::Value = serde_json::from_str(&json).expect("parse settings");
        let hooks = v.get("hooks").expect("hooks key");
        let ss = hooks
            .get("SessionStart")
            .and_then(serde_json::Value::as_array)
            .expect("SessionStart array");
        assert_eq!(ss.len(), 1);
        let entry = &ss[0];
        assert_eq!(
            entry.get("matcher").and_then(serde_json::Value::as_str),
            Some("*")
        );
        let cmd_arr = entry
            .get("hooks")
            .and_then(serde_json::Value::as_array)
            .expect("inner hooks array");
        let cmd = cmd_arr[0]
            .get("command")
            .and_then(serde_json::Value::as_str)
            .unwrap();
        assert!(cmd.starts_with("/tmp/hook.sh"));
        assert!(cmd.ends_with("SessionStart"));

        let stop = hooks
            .get("Stop")
            .and_then(serde_json::Value::as_array)
            .expect("Stop array");
        let stop_cmd = stop[0]
            .get("hooks")
            .and_then(serde_json::Value::as_array)
            .unwrap()[0]
            .get("command")
            .and_then(serde_json::Value::as_str)
            .unwrap();
        assert!(stop_cmd.ends_with("Stop"));
    }

    #[test]
    fn parse_line_well_formed() {
        let line = parse_line("Stop\t{\"transcript_path\":\"/x/y.jsonl\"}\n").unwrap();
        assert_eq!(line.event, HookEvent::Stop);
        assert_eq!(line.payload, "{\"transcript_path\":\"/x/y.jsonl\"}");
    }

    #[test]
    fn parse_line_session_start() {
        let line = parse_line("SessionStart\t{\"session_id\":\"abc\"}").unwrap();
        assert_eq!(line.event, HookEvent::SessionStart);
        assert_eq!(line.payload, "{\"session_id\":\"abc\"}");
    }

    #[test]
    fn parse_line_unknown_event_tagged() {
        let line = parse_line("PreToolUse\t{}").unwrap();
        assert_eq!(line.event, HookEvent::Unknown);
    }

    #[test]
    fn parse_line_no_tab_returns_none() {
        assert!(parse_line("no-tab-here").is_none());
    }

    #[test]
    fn extract_transcript_path_basic() {
        let p = extract_transcript_path(r#"{"transcript_path":"/a/b.jsonl","session_id":"x"}"#);
        assert_eq!(p.as_deref(), Some("/a/b.jsonl"));
    }

    #[test]
    fn extract_last_assistant_message_basic() {
        let m = extract_last_assistant_message(r#"{"last_assistant_message":"hello"}"#);
        assert_eq!(m.as_deref(), Some("hello"));
    }

    #[test]
    fn extract_session_id_basic() {
        let s = extract_session_id(r#"{"session_id":"abc-123"}"#);
        assert_eq!(s.as_deref(), Some("abc-123"));
    }

    #[test]
    fn extract_missing_field_returns_none() {
        let p = extract_transcript_path(r#"{"session_id":"x"}"#);
        assert!(p.is_none());
    }

    #[test]
    fn extract_invalid_json_returns_none() {
        assert!(extract_transcript_path("not json").is_none());
    }

    #[test]
    fn harness_create_then_drop_cleans_up() {
        let harness = HookHarness::create().expect("create harness");
        let tmp_dir = harness.tmp_dir.clone();
        let fifo = harness.fifo_path.clone();
        let script = harness.script_path.clone();

        assert!(tmp_dir.exists(), "tmp dir should exist after create");
        assert!(script.exists(), "script should exist after create");
        // FIFO exists; checking it doesn't require opening (which would block
        // since no writer is attached yet).
        let meta = std::fs::metadata(&fifo).expect("fifo metadata");
        use std::os::unix::fs::FileTypeExt;
        assert!(meta.file_type().is_fifo(), "fifo node should be FIFO");

        drop(harness);

        assert!(!fifo.exists(), "fifo should be removed on Drop");
        assert!(!script.exists(), "script should be removed on Drop");
        assert!(!tmp_dir.exists(), "tmp dir should be removed on Drop");
    }

    #[tokio::test]
    async fn reader_round_trips_a_line_written_to_fifo() {
        let harness = HookHarness::create().expect("create harness");
        let token = CancellationToken::new();
        let (tx, mut rx) = mpsc::unbounded_channel();

        let _handle =
            spawn_reader(harness.fifo_path.clone(), tx, token.clone()).expect("spawn reader");

        // Write one event line via the relay script (simulating claude firing
        // the hook). The script opens the FIFO for write and appends a line.
        let fifo = harness.fifo_path.clone();
        tokio::task::spawn_blocking(move || {
            use std::io::Write;
            // Open for write — blocks until reader is attached, but our reader
            // is already up.
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .open(&fifo)
                .expect("open fifo for write");
            f.write_all(b"Stop\t{\"transcript_path\":\"/tmp/x.jsonl\"}\n")
                .expect("write to fifo");
        })
        .await
        .expect("join writer");

        let line = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("recv timeout")
            .expect("recv None");
        assert_eq!(line.event, HookEvent::Stop);
        assert!(line.payload.contains("transcript_path"));

        token.cancel();
    }
}
