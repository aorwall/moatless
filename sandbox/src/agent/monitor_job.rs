//! Sandbox-local cron for the Claude Code `Monitor` tool.
//!
//! When the agent invokes `Monitor`, `InteractivePermissionHandler` rewrites
//! the tool input to a no-op (so the SDK's built-in implementation exits
//! immediately) and registers a `MonitorJob` against the shared
//! `ScheduledJobRegistry`. The job runs the supplied shell command on a fixed
//! 60s interval and stops once the command exits 0 with non-empty stdout, or
//! once `timeout_ms` elapses. Each terminal result is injected back into the
//! running Claude session as a `<task-notification>` user message that
//! mirrors the real Monitor envelope.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::process::Command;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use super::scheduled_job::{
    JobContext, JobInjector, JobKind, JobSnapshot, MonitorSnapshot, ScheduledJob,
};

/// Polling interval — every tick we run the registered command once.
const TICK_INTERVAL: Duration = Duration::from_secs(60);
/// Per-tick command timeout. Less than `TICK_INTERVAL` so tick N+1 never
/// races with tick N's child process.
const TICK_TIMEOUT: Duration = Duration::from_secs(50);
/// Maximum bytes of stdout we forward inline in the `<event>` block.
const MAX_INLINE_OUTPUT: usize = 16 * 1024;
/// Hard cap on the registered Monitor's deadline. Mirrors the SDK schema's
/// `timeout_ms` maximum (3 600 000 ms).
const MAX_TIMEOUT: Duration = Duration::from_secs(3600);
/// Where truncated payloads are written for the agent to fetch out of band.
const OVERFLOW_LOG_DIR: &str = "/tmp";

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

/// Cron job driving one Monitor tool invocation.
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

/// Worker loop for one registered Monitor.
async fn run_monitor_job(
    req: MonitorRequest,
    cron_id: String,
    injector: Arc<dyn JobInjector>,
    job_cancel: CancellationToken,
    run_cancel: CancellationToken,
) {
    let timeout = Duration::from_millis(req.timeout_ms).min(MAX_TIMEOUT);
    let deadline = Instant::now() + timeout;
    let mut interval = tokio::time::interval(TICK_INTERVAL);
    // First tick fires immediately — match what a user would expect from
    // "run this on a schedule" without an extra 60s delay before the first run.
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        // Wait for either the next scheduled tick, the deadline, or a cancel.
        tokio::select! {
            biased;
            () = job_cancel.cancelled() => {
                debug!(cron_id = %cron_id, "Monitor cancelled (job token)");
                return;
            }
            () = run_cancel.cancelled() => {
                debug!(cron_id = %cron_id, "Monitor cancelled (run token)");
                return;
            }
            () = tokio::time::sleep_until(deadline), if !req.persistent => {
                let notification = format_timeout(&cron_id, &req);
                injector.inject(notification);
                info!(cron_id = %cron_id, "Monitor timed out, injected timeout notification");
                return;
            }
            _ = interval.tick() => {}
        }

        // Run the command, but keep watching for cancel/deadline so a long
        // command (e.g. `sleep 5`) doesn't pin the worker.
        let outcome = tokio::select! {
            biased;
            () = job_cancel.cancelled() => {
                debug!(cron_id = %cron_id, "Monitor cancelled mid-tick (job token)");
                return;
            }
            () = run_cancel.cancelled() => {
                debug!(cron_id = %cron_id, "Monitor cancelled mid-tick (run token)");
                return;
            }
            () = tokio::time::sleep_until(deadline), if !req.persistent => {
                let notification = format_timeout(&cron_id, &req);
                injector.inject(notification);
                info!(cron_id = %cron_id, "Monitor timed out mid-tick, injected timeout notification");
                return;
            }
            result = run_command_once(&req.command) => result,
        };

        match outcome {
            Ok(TickOutcome::SuccessWithOutput(stdout)) => {
                let notification = format_completed(&cron_id, &req, &stdout);
                injector.inject(notification);
                info!(cron_id = %cron_id, "Monitor completed, injected result");
                return;
            }
            Ok(TickOutcome::EmptyOrNonZero { exit_code, stderr_summary }) => {
                debug!(
                    cron_id = %cron_id,
                    exit_code = ?exit_code,
                    stderr = %stderr_summary,
                    "Monitor tick produced no terminal output, polling again"
                );
            }
            Err(error) => {
                warn!(cron_id = %cron_id, error = %error, "Monitor tick failed");
            }
        }
    }
}

/// What a single command tick produced.
enum TickOutcome {
    /// Exit 0 with non-empty stdout — the terminal event we forward.
    SuccessWithOutput(String),
    /// Either non-zero exit, empty stdout, or both. Keep polling.
    EmptyOrNonZero {
        exit_code: Option<i32>,
        stderr_summary: String,
    },
}

async fn run_command_once(command: &str) -> Result<TickOutcome, std::io::Error> {
    let child = Command::new("/bin/sh")
        .arg("-c")
        .arg(command)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;

    let wait = child.wait_with_output();
    let output = match tokio::time::timeout(TICK_TIMEOUT, wait).await {
        Ok(result) => result?,
        Err(_) => {
            // wait_with_output() owns the child; on timeout it gets dropped
            // here and `kill_on_drop(true)` reaps it.
            return Ok(TickOutcome::EmptyOrNonZero {
                exit_code: None,
                stderr_summary: format!("tick exceeded {}s", TICK_TIMEOUT.as_secs()),
            });
        }
    };

    let exit_code = output.status.code();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if output.status.success() && !stdout.trim().is_empty() {
        Ok(TickOutcome::SuccessWithOutput(stdout))
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr_summary = stderr.lines().take(3).collect::<Vec<_>>().join(" | ");
        Ok(TickOutcome::EmptyOrNonZero {
            exit_code,
            stderr_summary,
        })
    }
}

/// Format the `<task-notification>` envelope emitted on successful completion.
fn format_completed(cron_id: &str, req: &MonitorRequest, stdout: &str) -> String {
    use std::fmt::Write as _;
    let (event_text, overflow_path) = prepare_event_text(cron_id, stdout);
    let mut buf = String::with_capacity(event_text.len() + 256);
    buf.push_str("<task-notification>\n");
    let _ = writeln!(buf, "<task-id>{cron_id}</task-id>");
    let _ = writeln!(buf, "<tool-use-id>{}</tool-use-id>", req.tool_use_id);
    let _ = writeln!(
        buf,
        "<summary>Monitor cron \"{}\" produced output</summary>",
        escape_xml(&req.description)
    );
    let _ = writeln!(buf, "<event>{}</event>", escape_xml(&event_text));
    if let Some(path) = overflow_path {
        let _ = writeln!(buf, "<output-file>{path}</output-file>");
    }
    buf.push_str("<status>completed</status>\n");
    buf.push_str("</task-notification>");
    buf
}

/// Format the timeout envelope.
fn format_timeout(cron_id: &str, req: &MonitorRequest) -> String {
    format!(
        "<task-notification>\n<task-id>{cron_id}</task-id>\n<tool-use-id>{tool_use_id}</tool-use-id>\n<summary>Monitor cron \"{description}\" timed out after {ms}ms with no successful output</summary>\n<status>timeout</status>\n</task-notification>",
        cron_id = cron_id,
        tool_use_id = req.tool_use_id,
        description = escape_xml(&req.description),
        ms = req.timeout_ms,
    )
}

/// Truncate stdout to `MAX_INLINE_OUTPUT` and, on overflow, persist the full
/// payload under `/tmp/monitor-{cron_id}.log` for the agent to fetch via Read
/// or cat.
fn prepare_event_text(cron_id: &str, stdout: &str) -> (String, Option<String>) {
    if stdout.len() <= MAX_INLINE_OUTPUT {
        return (stdout.to_string(), None);
    }
    let path = PathBuf::from(OVERFLOW_LOG_DIR).join(format!("monitor-{cron_id}.log"));
    if let Err(e) = std::fs::write(&path, stdout) {
        warn!(error = %e, path = %path.display(), "Failed to persist truncated Monitor output");
    }
    let truncated = stdout
        .char_indices()
        .nth(MAX_INLINE_OUTPUT)
        .map_or(stdout, |(i, _)| &stdout[..i]);
    let suffix = format!(
        "\n…(truncated, full {} bytes at {})",
        stdout.len(),
        path.display()
    );
    let mut text = String::with_capacity(truncated.len() + suffix.len());
    text.push_str(truncated);
    text.push_str(&suffix);
    (text, Some(path.display().to_string()))
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

/// Truncate a string to `max` characters on a char boundary.
fn truncate(input: &str, max: usize) -> String {
    match input.char_indices().nth(max) {
        Some((idx, _)) => format!("{}…", &input[..idx]),
        None => input.to_string(),
    }
}

/// Build the `Allow + updated_input` no-op rewrite that the permission handler
/// returns for Monitor calls. The CLI's built-in Monitor implementation will
/// see `command: ":"` (POSIX no-op), produce no events, and exit immediately.
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
    use std::sync::Mutex;

    use super::super::scheduled_job::ScheduledJobRegistry;
    use super::*;

    #[derive(Default, Clone)]
    struct CapturingInjector {
        messages: Arc<Mutex<Vec<String>>>,
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

    fn build_request(command: &str, timeout_ms: u64) -> MonitorRequest {
        MonitorRequest {
            tool_use_id: "toolu_abc123def456".to_string(),
            command: command.to_string(),
            description: "test".to_string(),
            timeout_ms,
            persistent: false,
        }
    }

    /// Wait until `predicate` returns true or `timeout` elapses. Used to bound
    /// async tests that spin up tokio tasks.
    async fn wait_until<F>(timeout: Duration, mut predicate: F) -> bool
    where
        F: FnMut() -> bool,
    {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if predicate() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        predicate()
    }

    #[tokio::test]
    async fn monitor_completes_on_zero_exit_with_output() {
        let injector = CapturingInjector::default();
        let registry = ScheduledJobRegistry::new(Arc::new(injector.clone()));
        let run_cancel = CancellationToken::new();

        // First tick fires immediately and `echo hello` produces stdout.
        registry
            .register(
                Box::new(MonitorJob::from_request(build_request("echo hello", 60_000))),
                run_cancel,
            )
            .await;

        let ok = wait_until(Duration::from_secs(5), || !injector.snapshot().is_empty()).await;
        assert!(ok, "expected an injected notification");

        let messages = injector.snapshot();
        assert_eq!(messages.len(), 1, "exactly one notification expected");
        let msg = &messages[0];
        assert!(msg.contains("<status>completed</status>"), "{msg}");
        assert!(msg.contains("hello"), "{msg}");
        assert!(msg.contains("<tool-use-id>toolu_abc123def456</tool-use-id>"), "{msg}");
        assert!(registry.is_empty().await);
    }

    #[tokio::test]
    async fn monitor_emits_timeout_notification() {
        let injector = CapturingInjector::default();
        let registry = ScheduledJobRegistry::new(Arc::new(injector.clone()));
        let run_cancel = CancellationToken::new();

        // Command always exits 0 but produces no output → keeps polling
        // until `timeout_ms` elapses.
        registry
            .register(
                Box::new(MonitorJob::from_request(build_request(":", 200))),
                run_cancel,
            )
            .await;

        let ok = wait_until(Duration::from_secs(5), || !injector.snapshot().is_empty()).await;
        assert!(ok, "expected a timeout notification");

        let messages = injector.snapshot();
        assert_eq!(messages.len(), 1);
        assert!(messages[0].contains("<status>timeout</status>"), "{}", messages[0]);
    }

    #[tokio::test]
    async fn monitor_cancels_on_run_token() {
        let injector = CapturingInjector::default();
        let registry = ScheduledJobRegistry::new(Arc::new(injector.clone()));
        let run_cancel = CancellationToken::new();

        // Long timeout, but run token cancels before the first tick can finish.
        registry
            .register(
                Box::new(MonitorJob::from_request(build_request(
                    "sleep 5; echo done",
                    60_000,
                ))),
                run_cancel.clone(),
            )
            .await;

        // Give the worker a moment to start, then cancel.
        tokio::time::sleep(Duration::from_millis(50)).await;
        run_cancel.cancel();

        // Cleanup is async — poll until the registry empties or we give up.
        let mut cleared = false;
        for _ in 0..50 {
            if registry.is_empty().await {
                cleared = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(cleared, "registry should be empty after cancellation");
        assert!(
            injector.snapshot().is_empty(),
            "no notification expected after cancel"
        );
    }

    #[tokio::test]
    async fn list_returns_snapshots_for_registered_jobs() {
        let injector = CapturingInjector::default();
        let registry = ScheduledJobRegistry::new(Arc::new(injector));
        let run_cancel = CancellationToken::new();

        // Long-running command so the job stays registered while we read.
        let mut req = build_request("sleep 30", 60_000);
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
        // Long commands are truncated for display.
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
        original.insert(
            "command".to_string(),
            Value::String("rm -rf /".to_string()),
        );
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
        assert_eq!(updated.get("persistent").and_then(Value::as_bool), Some(false));
    }

    #[test]
    fn xml_escaping_protects_event_block() {
        let req = MonitorRequest {
            tool_use_id: "toolu_x".to_string(),
            command: ":".to_string(),
            description: "<bad> & evil".to_string(),
            timeout_ms: 1000,
            persistent: false,
        };
        let out = format_completed("cron-test", &req, "raw <html> & stuff");
        assert!(out.contains("&lt;bad&gt; &amp; evil"));
        assert!(out.contains("raw &lt;html&gt; &amp; stuff"));
    }

    #[test]
    fn truncates_large_output_and_records_overflow_path() {
        // 32 KiB of x's
        let big = "x".repeat(32 * 1024);
        let (text, path) = prepare_event_text("cron-trunc", &big);
        assert!(text.len() < big.len());
        assert!(text.contains("(truncated"));
        assert!(path.is_some());
        // Cleanup
        if let Some(ref p) = path {
            let _ = std::fs::remove_file(p);
        }
    }

    #[test]
    fn short_id_strips_toolu_prefix() {
        assert_eq!(short_id("toolu_abc123"), "cron-abc123");
        assert_eq!(short_id("plain"), "cron-plain");
    }
}
