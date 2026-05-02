//! Sandbox-local streaming watcher for the Claude Code `Monitor` tool.
//!
//! When the agent invokes `Monitor`, `InteractivePermissionHandler` rewrites
//! the tool input to a no-op (so the SDK's built-in implementation exits
//! immediately) and registers a `MonitorJob` against the shared
//! `ScheduledJobRegistry`. The job spawns the supplied shell command **once**
//! as a long-running process and streams its stdout back to the running
//! Claude session as `<task-notification>` user messages — one notification
//! per 200ms-batched group of lines, plus a terminal envelope on process
//! exit, timeout, or cancellation. Stderr is captured to a per-job log file
//! (with `[stderr] ` prefix) but never produces an event; the agent reads
//! that file via the `Read` tool when stderr matters.
//!
//! Behavior matches the hosted Claude Code Monitor (verified empirically):
//! `persistent: false` runs until the process exits or `timeout_ms` elapses
//! (it does NOT cut off after the first event); `persistent: true` ignores
//! `timeout_ms` and runs until the process exits or the agent run ends.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::fs::OpenOptions;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, Mutex};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use super::scheduled_job::{
    JobContext, JobInjector, JobKind, JobSnapshot, MonitorSnapshot, ScheduledJob,
};

/// Stdout lines arriving within this window coalesce into a single
/// `<task-notification>` event. Matches the hosted Monitor batching window.
const BATCH_WINDOW: Duration = Duration::from_millis(200);
/// Maximum bytes of stdout we forward inline in the `<event>` block. Anything
/// over this is truncated and the agent is pointed at the output file.
const MAX_INLINE_OUTPUT: usize = 16 * 1024;
/// Hard cap on the registered Monitor's deadline. Mirrors the SDK schema's
/// `timeout_ms` maximum (3 600 000 ms).
const MAX_TIMEOUT: Duration = Duration::from_secs(3600);
/// Where the per-job stdout/stderr log is persisted. The agent reads this via
/// the `Read` tool when it needs the full stream or stderr.
const OUTPUT_LOG_DIR: &str = "/tmp";
/// Prefix applied to stderr lines in the output file. Matches the hosted
/// Monitor's format so agents have one consistent convention.
const STDERR_PREFIX: &str = "[stderr] ";

/// Parsed Monitor tool input.
#[derive(Debug, Clone)]
pub struct MonitorRequest {
    pub tool_use_id: String,
    pub command: String,
    pub description: String,
    pub timeout_ms: u64,
    pub persistent: bool,
}

impl MonitorRequest {
    /// Parse a `Monitor` tool input map. Returns `None` when required fields
    /// are missing — the permission handler then falls back to allowing the
    /// SDK's own validation to report the error.
    pub fn from_tool_input(
        tool_use_id: &str,
        input: &serde_json::Map<String, Value>,
    ) -> Option<Self> {
        let command = input.get("command")?.as_str()?.to_string();
        let description = input
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("(no description)")
            .to_string();
        let timeout_ms = input
            .get("timeout_ms")
            .and_then(Value::as_u64)
            .unwrap_or(300_000);
        let persistent = input
            .get("persistent")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        Some(MonitorRequest {
            tool_use_id: tool_use_id.to_string(),
            command,
            description,
            timeout_ms,
            persistent,
        })
    }
}

/// Streaming watcher driving one Monitor tool invocation.
pub struct MonitorJob {
    request: MonitorRequest,
    cron_id: String,
    registered_at_unix_ms: i64,
}

impl MonitorJob {
    pub fn from_request(request: MonitorRequest) -> Self {
        let cron_id = short_id(&request.tool_use_id);
        let registered_at_unix_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        Self {
            request,
            cron_id,
            registered_at_unix_ms,
        }
    }
}

#[async_trait::async_trait]
impl ScheduledJob for MonitorJob {
    fn id(&self) -> &str {
        &self.request.tool_use_id
    }

    fn kind(&self) -> JobKind {
        JobKind::Monitor
    }

    fn snapshot(&self) -> JobSnapshot {
        JobSnapshot::Monitor(MonitorSnapshot {
            id: self.request.tool_use_id.clone(),
            registered_at_unix_ms: self.registered_at_unix_ms,
            description: self.request.description.clone(),
            command_preview: truncate(&self.request.command, 200),
            timeout_ms: self.request.timeout_ms,
            persistent: self.request.persistent,
        })
    }

    async fn run(self: Box<Self>, ctx: JobContext) {
        run_monitor_job(
            self.request,
            self.cron_id,
            ctx.injector,
            ctx.job_cancel,
            ctx.run_cancel,
        )
        .await;
    }
}

/// Terminal outcome for one Monitor run. `Cancelled` is the only variant that
/// suppresses the terminal envelope — it matches today's "silent stop on
/// agent run end" behavior.
#[derive(Debug)]
enum TerminalStatus {
    Completed,
    Failed { exit_code: Option<i32> },
    Timeout,
    Cancelled,
}

/// Streaming watcher for one registered Monitor.
async fn run_monitor_job(
    req: MonitorRequest,
    cron_id: String,
    injector: Arc<dyn JobInjector>,
    job_cancel: CancellationToken,
    run_cancel: CancellationToken,
) {
    let output_path = output_log_path(&cron_id);

    // Open the output log truncating. If this fails the streaming events
    // still flow; the terminal envelope's `<output-file>` reference will
    // dangle, which the agent will discover when it tries to Read it.
    let output_file = match OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&output_path)
        .await
    {
        Ok(f) => Some(Arc::new(Mutex::new(f))),
        Err(e) => {
            warn!(
                error = %e,
                path = %output_path.display(),
                cron_id = %cron_id,
                "Failed to open Monitor output log; continuing without disk capture",
            );
            None
        }
    };

    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg(&req.command)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);

    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(e) => {
            warn!(
                cron_id = %cron_id,
                error = %e,
                "Failed to spawn Monitor command",
            );
            let term = format_terminal(
                &cron_id,
                &req,
                &output_path,
                TerminalStatus::Failed { exit_code: None },
            );
            injector.inject(term);
            return;
        }
    };

    let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
        // Unreachable in practice — both pipes were configured Stdio::piped()
        // immediately above — but a panic here would crash the agent run.
        warn!(
            cron_id = %cron_id,
            "Monitor child spawned without stdout/stderr pipes; aborting watcher",
        );
        let _ = child.start_kill();
        let _ = child.wait().await;
        let term = format_terminal(
            &cron_id,
            &req,
            &output_path,
            TerminalStatus::Failed { exit_code: None },
        );
        injector.inject(term);
        return;
    };

    let (line_tx, mut line_rx) = mpsc::unbounded_channel::<String>();

    // Stdout reader: forward every line to the event channel AND mirror to
    // the output file.
    let stdout_writer = output_file.clone();
    let stdout_handle = tokio::spawn(async move {
        let mut reader = BufReader::new(stdout).lines();
        loop {
            match reader.next_line().await {
                Ok(Some(line)) => {
                    if let Some(file) = &stdout_writer {
                        let mut guard = file.lock().await;
                        let _ = guard.write_all(line.as_bytes()).await;
                        let _ = guard.write_all(b"\n").await;
                        let _ = guard.flush().await;
                    }
                    if line_tx.send(line).is_err() {
                        // Worker has gone away — stop reading.
                        break;
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    warn!(error = %e, "Monitor stdout read error");
                    break;
                }
            }
        }
    });

    // Stderr reader: writes to the output file, never produces events.
    let stderr_writer = output_file.clone();
    let stderr_handle = tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        loop {
            match reader.next_line().await {
                Ok(Some(line)) => {
                    if let Some(file) = &stderr_writer {
                        let mut guard = file.lock().await;
                        let _ = guard.write_all(STDERR_PREFIX.as_bytes()).await;
                        let _ = guard.write_all(line.as_bytes()).await;
                        let _ = guard.write_all(b"\n").await;
                        let _ = guard.flush().await;
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    warn!(error = %e, "Monitor stderr read error");
                    break;
                }
            }
        }
    });

    let timeout_at: Option<Instant> = if req.persistent {
        None
    } else {
        Some(Instant::now() + Duration::from_millis(req.timeout_ms).min(MAX_TIMEOUT))
    };

    let mut buffer: Vec<String> = Vec::new();
    let mut buffer_bytes: usize = 0;
    let mut flush_at: Option<Instant> = None;
    // Once the stdout reader closes line_tx, recv() returns None forever;
    // without a guard the biased select would starve child.wait().
    let mut stdout_closed = false;

    info!(
        cron_id = %cron_id,
        persistent = req.persistent,
        timeout_ms = req.timeout_ms,
        "Monitor started",
    );

    let terminal: TerminalStatus = loop {
        tokio::select! {
            biased;

            () = job_cancel.cancelled() => {
                debug!(cron_id = %cron_id, "Monitor cancelled (job token)");
                break TerminalStatus::Cancelled;
            }
            () = run_cancel.cancelled() => {
                debug!(cron_id = %cron_id, "Monitor cancelled (run token)");
                break TerminalStatus::Cancelled;
            }

            () = sleep_until_or_pending(timeout_at) => {
                debug!(cron_id = %cron_id, "Monitor timeout reached");
                if !buffer.is_empty() {
                    flush_buffer(
                        &mut buffer,
                        &mut buffer_bytes,
                        &cron_id,
                        &req.description,
                        &output_path,
                        injector.as_ref(),
                    );
                }
                break TerminalStatus::Timeout;
            }

            () = sleep_until_or_pending(flush_at) => {
                flush_at = None;
                if !buffer.is_empty() {
                    flush_buffer(
                        &mut buffer,
                        &mut buffer_bytes,
                        &cron_id,
                        &req.description,
                        &output_path,
                        injector.as_ref(),
                    );
                }
            }

            line = line_rx.recv(), if !stdout_closed => {
                match line {
                    Some(line) => {
                        buffer_bytes += line.len() + 1;
                        buffer.push(line);
                        if buffer_bytes >= MAX_INLINE_OUTPUT {
                            flush_buffer(
                                &mut buffer,
                                &mut buffer_bytes,
                                &cron_id,
                                &req.description,
                                &output_path,
                                injector.as_ref(),
                            );
                            flush_at = None;
                        } else if flush_at.is_none() {
                            flush_at = Some(Instant::now() + BATCH_WINDOW);
                        }
                    }
                    None => {
                        // stdout reader finished. Disarm this arm and wait
                        // for child.wait to fire; its arm handles the final
                        // drain + flush + status mapping.
                        stdout_closed = true;
                    }
                }
            }

            status = child.wait() => {
                // Drain anything the reader has already pushed but we
                // haven't pulled yet, then await the reader so no tail
                // line is missed.
                while let Ok(line) = line_rx.try_recv() {
                    buffer_bytes += line.len() + 1;
                    buffer.push(line);
                }
                let _ = stdout_handle.await;
                while let Ok(line) = line_rx.try_recv() {
                    buffer_bytes += line.len() + 1;
                    buffer.push(line);
                }
                if !buffer.is_empty() {
                    flush_buffer(
                        &mut buffer,
                        &mut buffer_bytes,
                        &cron_id,
                        &req.description,
                        &output_path,
                        injector.as_ref(),
                    );
                }
                break match status {
                    Ok(exit) if exit.success() => TerminalStatus::Completed,
                    Ok(exit) => TerminalStatus::Failed { exit_code: exit.code() },
                    Err(e) => {
                        warn!(cron_id = %cron_id, error = %e, "Monitor child wait failed");
                        TerminalStatus::Failed { exit_code: None }
                    }
                };
            }
        }
    };

    match terminal {
        TerminalStatus::Cancelled => {
            // Match today's behavior: cancel = silent stop, no envelope.
            let _ = child.start_kill();
            let _ = child.wait().await;
            let _ = stderr_handle.await;
            return;
        }
        TerminalStatus::Timeout => {
            let _ = child.start_kill();
            let _ = child.wait().await;
        }
        _ => {}
    }

    // Wait for the stderr reader to finish flushing whatever was in flight.
    // (The stdout reader was already awaited inside the child.wait arm.)
    let _ = stderr_handle.await;

    info!(
        cron_id = %cron_id,
        ?terminal,
        "Monitor terminated",
    );

    let term = format_terminal(&cron_id, &req, &output_path, terminal);
    injector.inject(term);
}

/// Sleep until the deadline, or never if `deadline` is `None`. Lets a
/// `select!` arm be conditionally armed without juggling `Box::pin`.
async fn sleep_until_or_pending(deadline: Option<Instant>) {
    match deadline {
        Some(t) => tokio::time::sleep_until(t).await,
        None => std::future::pending().await,
    }
}

/// Build and inject a streaming `<task-notification>` event from the buffered
/// stdout lines, then clear the buffer.
fn flush_buffer(
    buffer: &mut Vec<String>,
    buffer_bytes: &mut usize,
    cron_id: &str,
    description: &str,
    output_path: &Path,
    injector: &dyn JobInjector,
) {
    if buffer.is_empty() {
        return;
    }
    let body = buffer.join("\n");
    buffer.clear();
    *buffer_bytes = 0;
    let envelope = format_streaming_event(cron_id, description, &body, output_path);
    injector.inject(envelope);
}

/// Format the streaming `<task-notification>` for one batch of stdout lines.
fn format_streaming_event(
    cron_id: &str,
    description: &str,
    body: &str,
    output_path: &Path,
) -> String {
    use std::fmt::Write as _;
    let inline = truncate_event_body(body, output_path);
    let mut buf = String::with_capacity(inline.len() + 256);
    buf.push_str("<task-notification>\n");
    let _ = writeln!(buf, "<task-id>{cron_id}</task-id>");
    let _ = writeln!(
        buf,
        "<summary>Monitor event: \"{}\"</summary>",
        escape_xml(description)
    );
    let _ = writeln!(buf, "<event>{}</event>", escape_xml(&inline));
    buf.push_str("</task-notification>");
    buf
}

/// Format the terminal `<task-notification>` (completed / failed / timeout).
fn format_terminal(
    cron_id: &str,
    req: &MonitorRequest,
    output_path: &Path,
    status: TerminalStatus,
) -> String {
    use std::fmt::Write as _;
    let (tag, summary) = match status {
        TerminalStatus::Completed => (
            "completed",
            format!(
                "Monitor \"{}\" stream ended",
                escape_xml(&req.description)
            ),
        ),
        TerminalStatus::Failed {
            exit_code: Some(code),
        } => (
            "failed",
            format!(
                "Monitor \"{}\" script failed (exit {code})",
                escape_xml(&req.description)
            ),
        ),
        TerminalStatus::Failed { exit_code: None } => (
            "failed",
            format!(
                "Monitor \"{}\" script killed by signal",
                escape_xml(&req.description)
            ),
        ),
        TerminalStatus::Timeout => (
            "timeout",
            format!(
                "Monitor \"{}\" timed out after {}ms",
                escape_xml(&req.description),
                req.timeout_ms
            ),
        ),
        TerminalStatus::Cancelled => unreachable!("Cancelled never produces a terminal envelope"),
    };
    let mut buf = String::with_capacity(summary.len() + 256);
    buf.push_str("<task-notification>\n");
    let _ = writeln!(buf, "<task-id>{cron_id}</task-id>");
    let _ = writeln!(buf, "<tool-use-id>{}</tool-use-id>", req.tool_use_id);
    let _ = writeln!(buf, "<output-file>{}</output-file>", output_path.display());
    let _ = writeln!(buf, "<status>{tag}</status>");
    let _ = writeln!(buf, "<summary>{summary}</summary>");
    buf.push_str("</task-notification>");
    buf
}

/// Truncate event body to `MAX_INLINE_OUTPUT` bytes; on overflow, append a
/// pointer to the (already-being-written) output file.
fn truncate_event_body(body: &str, output_path: &Path) -> String {
    if body.len() <= MAX_INLINE_OUTPUT {
        return body.to_string();
    }
    let truncated = body
        .char_indices()
        .nth(MAX_INLINE_OUTPUT)
        .map_or(body, |(i, _)| &body[..i]);
    format!(
        "{truncated}\n…(truncated, full {} bytes at {})",
        body.len(),
        output_path.display()
    )
}

fn output_log_path(cron_id: &str) -> PathBuf {
    PathBuf::from(OUTPUT_LOG_DIR).join(format!("monitor-{cron_id}.log"))
}

fn short_id(tool_use_id: &str) -> String {
    let trimmed = tool_use_id.strip_prefix("toolu_").unwrap_or(tool_use_id);
    let suffix: String = trimmed.chars().take(10).collect();
    format!("cron-{suffix}")
}

fn escape_xml(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            other => out.push(other),
        }
    }
    out
}

fn truncate(input: &str, max: usize) -> String {
    match input.char_indices().nth(max) {
        Some((idx, _)) => format!("{}…", &input[..idx]),
        None => input.to_string(),
    }
}

/// Build the `Allow + updated_input` no-op rewrite that the permission handler
/// returns for Monitor calls. The CLI's built-in Monitor implementation will
/// see `command: ":"` (POSIX no-op), produce no events, and exit immediately;
/// the streaming events arrive later from `MonitorJob`.
pub fn noop_monitor_input(
    original: &serde_json::Map<String, Value>,
) -> serde_json::Map<String, Value> {
    let mut updated = original.clone();
    updated.insert("command".to_string(), Value::String(":".to_string()));
    updated.insert(
        "timeout_ms".to_string(),
        Value::Number(serde_json::Number::from(1000u64)),
    );
    updated.insert("persistent".to_string(), Value::Bool(false));
    updated
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex as StdMutex;

    use super::super::scheduled_job::ScheduledJobRegistry;
    use super::*;

    #[derive(Default, Clone)]
    struct CapturingInjector {
        messages: Arc<StdMutex<Vec<String>>>,
    }

    impl CapturingInjector {
        fn snapshot(&self) -> Vec<String> {
            self.messages.lock().expect("mutex poisoned").clone()
        }
    }

    impl JobInjector for CapturingInjector {
        fn inject(&self, text: String) {
            self.messages.lock().expect("mutex poisoned").push(text);
        }
    }

    fn build_request(tool_use_id: &str, command: &str, timeout_ms: u64) -> MonitorRequest {
        MonitorRequest {
            tool_use_id: tool_use_id.to_string(),
            command: command.to_string(),
            description: "test".to_string(),
            timeout_ms,
            persistent: false,
        }
    }

    /// Wait until `predicate` returns true or `timeout` elapses.
    async fn wait_until<F>(timeout: Duration, mut predicate: F) -> bool
    where
        F: FnMut() -> bool,
    {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if predicate() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        predicate()
    }

    fn streaming_events(messages: &[String]) -> Vec<&str> {
        messages
            .iter()
            .filter(|m| m.contains("<summary>Monitor event:"))
            .map(String::as_str)
            .collect()
    }

    fn terminal_events(messages: &[String]) -> Vec<&str> {
        messages
            .iter()
            .filter(|m| m.contains("<status>"))
            .map(String::as_str)
            .collect()
    }

    #[tokio::test]
    async fn burst_lines_coalesce_into_one_event() {
        let injector = CapturingInjector::default();
        let registry = ScheduledJobRegistry::new(Arc::new(injector.clone()));
        let run_cancel = CancellationToken::new();

        registry
            .register(
                Box::new(MonitorJob::from_request(build_request(
                    "toolu_burst",
                    "for i in 1 2 3 4 5; do echo line$i; done; exit 0",
                    10_000,
                ))),
                run_cancel,
            )
            .await;

        // Wait for terminal envelope (proves the run completed).
        let ok = wait_until(Duration::from_secs(5), || {
            !terminal_events(&injector.snapshot()).is_empty()
        })
        .await;
        assert!(ok, "expected terminal envelope");

        let messages = injector.snapshot();
        let streaming = streaming_events(&messages);
        let terminal = terminal_events(&messages);
        assert_eq!(
            streaming.len(),
            1,
            "5 instant lines should coalesce into one streaming event; got {streaming:?}",
        );
        assert!(streaming[0].contains("line1"));
        assert!(streaming[0].contains("line5"));
        assert_eq!(terminal.len(), 1);
        assert!(terminal[0].contains("<status>completed</status>"));
        assert!(terminal[0].contains("<tool-use-id>toolu_burst</tool-use-id>"));
        assert!(terminal[0].contains("<output-file>/tmp/monitor-cron-burst.log</output-file>"));
        assert!(registry.is_empty().await);
    }

    #[tokio::test]
    async fn spaced_lines_emit_separate_events() {
        let injector = CapturingInjector::default();
        let registry = ScheduledJobRegistry::new(Arc::new(injector.clone()));
        let run_cancel = CancellationToken::new();

        registry
            .register(
                Box::new(MonitorJob::from_request(build_request(
                    "toolu_spaced",
                    "for i in 1 2 3; do echo tick$i; sleep 0.4; done; exit 0",
                    10_000,
                ))),
                run_cancel,
            )
            .await;

        let ok = wait_until(Duration::from_secs(5), || {
            !terminal_events(&injector.snapshot()).is_empty()
        })
        .await;
        assert!(ok, "expected terminal envelope");

        let messages = injector.snapshot();
        let streaming = streaming_events(&messages);
        assert_eq!(
            streaming.len(),
            3,
            "3 lines spaced 400ms apart should emit 3 separate events; got {streaming:?}",
        );
        assert!(streaming[0].contains("tick1"));
        assert!(streaming[1].contains("tick2"));
        assert!(streaming[2].contains("tick3"));
        let terminal = terminal_events(&messages);
        assert_eq!(terminal.len(), 1);
        assert!(terminal[0].contains("<status>completed</status>"));
    }

    #[tokio::test]
    async fn silent_exit_emits_completed_only() {
        let injector = CapturingInjector::default();
        let registry = ScheduledJobRegistry::new(Arc::new(injector.clone()));
        let run_cancel = CancellationToken::new();

        registry
            .register(
                Box::new(MonitorJob::from_request(build_request(
                    "toolu_silent",
                    "true",
                    10_000,
                ))),
                run_cancel,
            )
            .await;

        let ok = wait_until(Duration::from_secs(5), || {
            !terminal_events(&injector.snapshot()).is_empty()
        })
        .await;
        assert!(ok, "expected terminal envelope");

        let messages = injector.snapshot();
        assert!(
            streaming_events(&messages).is_empty(),
            "silent exit should not produce any streaming event; got {messages:?}",
        );
        let terminal = terminal_events(&messages);
        assert_eq!(terminal.len(), 1);
        assert!(terminal[0].contains("<status>completed</status>"));
    }

    #[tokio::test]
    async fn failed_exit_emits_failed_with_exit_code() {
        let injector = CapturingInjector::default();
        let registry = ScheduledJobRegistry::new(Arc::new(injector.clone()));
        let run_cancel = CancellationToken::new();

        registry
            .register(
                Box::new(MonitorJob::from_request(build_request(
                    "toolu_fail",
                    "exit 7",
                    10_000,
                ))),
                run_cancel,
            )
            .await;

        let ok = wait_until(Duration::from_secs(5), || {
            !terminal_events(&injector.snapshot()).is_empty()
        })
        .await;
        assert!(ok, "expected terminal envelope");

        let messages = injector.snapshot();
        let terminal = terminal_events(&messages);
        assert_eq!(terminal.len(), 1);
        assert!(terminal[0].contains("<status>failed</status>"));
        assert!(
            terminal[0].contains("(exit 7)"),
            "summary should include the exit code: {}",
            terminal[0]
        );
    }

    #[tokio::test]
    async fn stderr_goes_to_output_file_not_events() {
        let injector = CapturingInjector::default();
        let registry = ScheduledJobRegistry::new(Arc::new(injector.clone()));
        let run_cancel = CancellationToken::new();

        registry
            .register(
                Box::new(MonitorJob::from_request(build_request(
                    "toolu_stderr",
                    "echo good; echo bad 1>&2; exit 0",
                    10_000,
                ))),
                run_cancel,
            )
            .await;

        let ok = wait_until(Duration::from_secs(5), || {
            !terminal_events(&injector.snapshot()).is_empty()
        })
        .await;
        assert!(ok, "expected terminal envelope");

        let messages = injector.snapshot();
        let streaming = streaming_events(&messages);
        assert_eq!(streaming.len(), 1);
        assert!(streaming[0].contains("good"));
        assert!(
            !streaming[0].contains("bad"),
            "stderr should never appear in streaming events"
        );

        let log = std::fs::read_to_string("/tmp/monitor-cron-stderr.log")
            .expect("output log written");
        assert!(log.contains("good"), "stdout should be in output file: {log}");
        assert!(
            log.contains("[stderr] bad"),
            "stderr should be prefixed in output file: {log}",
        );
        let _ = std::fs::remove_file("/tmp/monitor-cron-stderr.log");
    }

    #[tokio::test]
    async fn persistent_false_streams_until_timeout() {
        let injector = CapturingInjector::default();
        let registry = ScheduledJobRegistry::new(Arc::new(injector.clone()));
        let run_cancel = CancellationToken::new();

        // Long-running emitter; timeout cuts it off.
        let mut req = build_request(
            "toolu_persfalse",
            "while true; do echo tick; sleep 0.1; done",
            500,
        );
        req.persistent = false;
        registry
            .register(Box::new(MonitorJob::from_request(req)), run_cancel)
            .await;

        let ok = wait_until(Duration::from_secs(5), || {
            !terminal_events(&injector.snapshot()).is_empty()
        })
        .await;
        assert!(ok, "expected terminal envelope");

        let messages = injector.snapshot();
        let streaming = streaming_events(&messages);
        assert!(
            !streaming.is_empty(),
            "persistent=false should still stream while running; got {messages:?}",
        );
        let terminal = terminal_events(&messages);
        assert_eq!(terminal.len(), 1);
        assert!(
            terminal[0].contains("<status>timeout</status>"),
            "terminal should be timeout (not completed): {}",
            terminal[0]
        );
    }

    #[tokio::test]
    async fn cancels_on_run_token_silently() {
        let injector = CapturingInjector::default();
        let registry = ScheduledJobRegistry::new(Arc::new(injector.clone()));
        let run_cancel = CancellationToken::new();

        registry
            .register(
                Box::new(MonitorJob::from_request(build_request(
                    "toolu_cancel",
                    "sleep 5",
                    60_000,
                ))),
                run_cancel.clone(),
            )
            .await;

        // Let the watcher start, then cancel before any output arrives.
        tokio::time::sleep(Duration::from_millis(50)).await;
        run_cancel.cancel();

        let mut empty = false;
        for _ in 0..100 {
            if registry.is_empty().await {
                empty = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(empty, "registry should empty after cancellation");
        assert!(
            injector.snapshot().is_empty(),
            "no envelope expected after cancel; got {:?}",
            injector.snapshot(),
        );
    }

    #[tokio::test]
    async fn list_returns_snapshots_for_registered_jobs() {
        let injector = CapturingInjector::default();
        let registry = ScheduledJobRegistry::new(Arc::new(injector));
        let run_cancel = CancellationToken::new();

        let mut req = build_request("toolu_listsnap", "sleep 30", 60_000);
        req.description = "argocd app sync for t-249273".to_string();
        req.command =
            "until kubectl get application soap-backend-preview-t-249273 -n argocd; do sleep 15; done"
                .to_string();
        registry
            .register(
                Box::new(MonitorJob::from_request(req.clone())),
                run_cancel.clone(),
            )
            .await;

        let snapshots = registry.list().await;
        assert_eq!(snapshots.len(), 1);
        let snap = match &snapshots[0] {
            JobSnapshot::Monitor(m) => m,
            other => panic!("expected monitor snapshot, got {other:?}"),
        };
        assert_eq!(snap.id, req.tool_use_id);
        assert_eq!(snap.description, "argocd app sync for t-249273");
        assert_eq!(snap.timeout_ms, 60_000);
        assert!(!snap.persistent);
        assert!(snap.registered_at_unix_ms > 0);
        assert!(
            snap.command_preview.contains("kubectl get application"),
            "preview should contain start of command: {}",
            snap.command_preview
        );
        run_cancel.cancel();
    }

    #[test]
    fn parse_tool_input_extracts_fields() {
        let mut input = serde_json::Map::new();
        input.insert("command".to_string(), Value::String("echo hi".to_string()));
        input.insert("description".to_string(), Value::String("d".to_string()));
        input.insert(
            "timeout_ms".to_string(),
            Value::Number(serde_json::Number::from(12_345u64)),
        );
        input.insert("persistent".to_string(), Value::Bool(true));

        let req = MonitorRequest::from_tool_input("toolu_x", &input).expect("parse");
        assert_eq!(req.command, "echo hi");
        assert_eq!(req.description, "d");
        assert_eq!(req.timeout_ms, 12_345);
        assert!(req.persistent);
    }

    #[test]
    fn parse_tool_input_requires_command() {
        let input = serde_json::Map::new();
        assert!(MonitorRequest::from_tool_input("toolu_x", &input).is_none());
    }

    #[test]
    fn noop_input_rewrites_command_and_timeout() {
        let mut original = serde_json::Map::new();
        original.insert("command".to_string(), Value::String("rm -rf /".to_string()));
        original.insert(
            "timeout_ms".to_string(),
            Value::Number(serde_json::Number::from(600_000u64)),
        );
        original.insert("persistent".to_string(), Value::Bool(true));

        let updated = noop_monitor_input(&original);
        assert_eq!(updated.get("command").and_then(Value::as_str), Some(":"));
        assert_eq!(
            updated.get("timeout_ms").and_then(Value::as_u64),
            Some(1000)
        );
        assert_eq!(
            updated.get("persistent").and_then(Value::as_bool),
            Some(false)
        );
    }

    #[test]
    fn streaming_event_escapes_xml_in_description_and_body() {
        let path = PathBuf::from("/tmp/monitor-cron-test.log");
        let out = format_streaming_event("cron-test", "<bad> & evil", "raw <html> & stuff", &path);
        assert!(out.contains("<task-id>cron-test</task-id>"));
        assert!(out.contains("&lt;bad&gt; &amp; evil"));
        assert!(out.contains("raw &lt;html&gt; &amp; stuff"));
        assert!(!out.contains("<status>"), "streaming events have no status");
        assert!(
            !out.contains("<tool-use-id>"),
            "streaming events have no tool-use-id"
        );
    }

    #[test]
    fn terminal_event_includes_tool_use_id_and_output_file() {
        let req = MonitorRequest {
            tool_use_id: "toolu_x".to_string(),
            command: ":".to_string(),
            description: "d".to_string(),
            timeout_ms: 1000,
            persistent: false,
        };
        let path = PathBuf::from("/tmp/monitor-cron-x.log");
        let out = format_terminal(
            "cron-x",
            &req,
            &path,
            TerminalStatus::Failed { exit_code: Some(7) },
        );
        assert!(out.contains("<task-id>cron-x</task-id>"));
        assert!(out.contains("<tool-use-id>toolu_x</tool-use-id>"));
        assert!(out.contains("<output-file>/tmp/monitor-cron-x.log</output-file>"));
        assert!(out.contains("<status>failed</status>"));
        assert!(out.contains("(exit 7)"));
    }

    #[test]
    fn truncates_large_event_body_and_points_at_output_file() {
        let big = "x".repeat(32 * 1024);
        let path = PathBuf::from("/tmp/monitor-cron-trunc.log");
        let body = truncate_event_body(&big, &path);
        assert!(body.len() < big.len());
        assert!(body.contains("(truncated"));
        assert!(body.contains("/tmp/monitor-cron-trunc.log"));
    }

    #[test]
    fn short_id_strips_toolu_prefix() {
        assert_eq!(short_id("toolu_abc123"), "cron-abc123");
        assert_eq!(short_id("plain"), "cron-plain");
    }
}
