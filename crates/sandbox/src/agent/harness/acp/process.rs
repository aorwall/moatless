use std::collections::{BTreeSet, HashSet, VecDeque};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::process::Child;
use tokio::time::Instant;

use super::AcpConfig;
use super::agent::{AcpAgent, task_token};
use super::gateway::GatewayProxy;
use super::journal::{AcpSessionFormat, Journal, bound_session, seed};
use super::rpc::{Incoming, Rpc};
use crate::agent::harness::content::CanonicalContentBlock as Block;
use crate::agent::harness::process::{AgentProcess, ProcessEvent, ProcessTermination};
use crate::agent::harness::signal::{Activity, ContextWindow, Input, RunTotals, SessionId};
use crate::agent::harness::{ErrorClass, FailureStage, HarnessError, SessionImpact};
use crate::agent::types::ResumeSession;
use crate::trajectory_tail::TurnTrajectory;

#[cfg(test)]
mod tests;

/// How long an ACP request during startup or a configuration change may take.
const CALL_TIMEOUT: Duration = Duration::from_secs(60);
/// How long the agent has to answer `session/cancel` before its tree is killed.
const CANCEL_GRACE: Duration = Duration::from_secs(10);
/// Quiet time after which an execution with no prompt outstanding ends.
const INTERNAL_QUIET: Duration = Duration::from_millis(if cfg!(test) { 300 } else { 3_000 });
/// How long an execution whose prompts returned waits for tool calls still
/// running; a backgrounded dev server would otherwise hold the turn forever.
const BACKGROUND_WAIT: Duration = Duration::from_secs(if cfg!(test) { 2 } else { 600 });

fn error(class: ErrorClass, diagnostic: impl ToString) -> HarnessError {
    HarnessError::new(
        FailureStage::Execution,
        class,
        SessionImpact::Intact,
        diagnostic.to_string(),
    )
}

/// A running agent process and the sessions it serves.
struct Live {
    child: Child,
    rpc: Rpc,
    acp_session: String,
    journal: Journal,
    model: Option<String>,
    mode: Option<String>,
    /// Names, not values: a value rotating under the same key must not respawn.
    env_keys: BTreeSet<String>,
    /// The transcript the next prompt carries, set when the agent session was
    /// opened fresh for a journal that already holds a conversation.
    seed: Option<String>,
}

/// The execution in progress.
#[derive(Default)]
struct Execution {
    /// `session/prompt` requests not yet answered; the execution ends when the
    /// last is.
    prompts: HashSet<u64>,
    internal: bool,
    /// Tool calls not yet `completed` or `failed`. agy ends a prompt while a
    /// long command runs in the background and wakes up when it finishes.
    running: HashSet<String>,
    background_until: Option<Instant>,
    quiet_until: Option<Instant>,
    kill_at: Option<Instant>,
    totals: Totals,
    failure: Option<HarnessError>,
    /// Assistant text and thought chunks awaiting their line.
    reply: String,
    thought: String,
    /// The whole reply of the execution, for a failure the agent wrote as text.
    replied: String,
}

impl Execution {
    /// Re-arms the end of an execution whose prompts have all returned.
    /// Returns true when it ends now.
    fn rearm(&mut self, now: Instant) -> bool {
        if !self.prompts.is_empty() {
            return false;
        }
        if !self.running.is_empty() {
            self.quiet_until = None;
            self.background_until.get_or_insert(now + BACKGROUND_WAIT);
            return false;
        }
        if self.internal || self.background_until.is_some() {
            self.quiet_until = Some(now + INTERNAL_QUIET);
            return false;
        }
        true
    }
}

#[derive(Default)]
struct Totals {
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
    thought: u64,
    prompts: u32,
}

/// A persistent ACP agent behind the harness seam.
pub(crate) struct AcpProcess {
    cfg: AcpConfig,
    agent: AcpAgent,
    live: Option<Live>,
    exec: Option<Execution>,
    trajectory: Option<TurnTrajectory>,
    gateway: Option<GatewayProxy>,
    pending: VecDeque<ProcessEvent>,
    /// Messages read while awaiting a response, handled by `next` in order.
    stash: VecDeque<Incoming>,
    /// Events released only after the trajectory flush that must precede them.
    after_flush: Option<Vec<ProcessEvent>>,
}

impl AcpProcess {
    pub(crate) fn new(cfg: AcpConfig) -> Self {
        Self {
            agent: AcpAgent::for_profile(cfg.profile).unwrap_or(AcpAgent::Antigravity),
            cfg,
            live: None,
            exec: None,
            trajectory: None,
            gateway: None,
            pending: VecDeque::new(),
            stash: VecDeque::new(),
            after_flush: None,
        }
    }

    /// Sends `method` and awaits its answer, stashing whatever else arrives.
    /// With `discard`, notifications are dropped instead: `session/load`
    /// replays the whole conversation as updates the journal already holds.
    async fn call(
        &mut self,
        method: &str,
        params: Value,
        discard: bool,
    ) -> Result<Value, HarnessError> {
        let live = self
            .live
            .as_mut()
            .ok_or_else(|| error(ErrorClass::ProcessExit, "ACP agent is not running"))?;
        let id = live.rpc.request(method, params);
        let deadline = Instant::now() + CALL_TIMEOUT;
        loop {
            let message = tokio::time::timeout_at(deadline, live.rpc.recv())
                .await
                .map_err(|_| {
                    error(
                        ErrorClass::UnresponsiveProcess,
                        format!("ACP `{method}` timed out"),
                    )
                })?
                .ok_or_else(|| {
                    error(
                        ErrorClass::ProcessExit,
                        format!("ACP agent exited during `{method}`"),
                    )
                })?;
            match message {
                Incoming::Response { id: got, result } if got == id => {
                    return result.map_err(|e| {
                        error(
                            ErrorClass::NativeProtocol,
                            format!("ACP `{method}` failed: {e}"),
                        )
                    });
                }
                Incoming::Notification { .. } if discard => {}
                Incoming::Request { id, method, params } if discard => {
                    answer_request(&live.rpc, id, &method, &params);
                }
                other => self.stash.push_back(other),
            }
        }
    }

    /// Whether `input` can run on the live agent rather than a new one.
    fn reusable(&self, input: &Input) -> bool {
        let Some(live) = &self.live else {
            return false;
        };
        let resumes_live = match &input.resume {
            Some(ResumeSession::WithId(id)) => *id == live.journal.id,
            _ => true,
        };
        input.fork.is_none()
            && !input.switch_harness
            && !input.start_fresh
            && resumes_live
            && input
                .env
                .as_ref()
                .is_none_or(|_| live.env_keys == env_keys(input))
    }

    /// The URL of this process's gateway proxy, started on first use.
    async fn gateway(&mut self) -> Result<String, HarnessError> {
        if self.gateway.is_none() {
            let base = self.cfg.config.backend_api_url.clone().ok_or_else(|| {
                error(
                    ErrorClass::InvalidConfiguration,
                    "ACP agents require the model gateway",
                )
            })?;
            self.gateway = Some(
                GatewayProxy::start(Arc::clone(&self.cfg.config), &base)
                    .await
                    .map_err(|e| error(ErrorClass::InvalidConfiguration, e))?,
            );
        }
        Ok(self.gateway.as_ref().map_or_default(GatewayProxy::url))
    }

    /// Spawns the agent and opens the journal `input` resumes, forks or starts.
    async fn open(&mut self, input: &Input) -> Result<(), HarnessError> {
        let config = Arc::clone(&self.cfg.config);
        let home = config.acp_home_dir();
        tokio::fs::create_dir_all(home.join("sessions"))
            .await
            .map_err(|e| error(ErrorClass::InvalidConfiguration, e))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700))
                .await
                .map_err(|e| error(ErrorClass::InvalidConfiguration, e))?;
        }
        self.trajectory = TurnTrajectory::new(
            self.cfg.trajectory.as_ref(),
            Arc::new(AcpSessionFormat),
            &config,
            &home.to_string_lossy(),
            input,
        );
        let restored = match &mut self.trajectory {
            Some(t) => t.write_out(input).await?,
            None => None,
        };
        if (input.fork.is_some() || input.switch_harness) && restored.is_none() {
            return Err(error(
                ErrorClass::UnavailableSession,
                "Could not restore the forked or switched conversation",
            ));
        }
        let resume = restored.clone().or_else(|| match &input.resume {
            Some(ResumeSession::WithId(id)) if !input.start_fresh => Some(id.clone()),
            _ => None,
        });
        let journal_id = resume
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        if uuid::Uuid::parse_str(&journal_id).is_err() {
            return Err(error(
                ErrorClass::UnavailableSession,
                "Invalid ACP session id",
            ));
        }
        if resume.is_some() && !super::journal::journal_path(&home, &journal_id).is_file() {
            return Err(error(
                ErrorClass::UnavailableSession,
                "ACP session journal is missing",
            ));
        }
        let (journal, existing) = Journal::open(&home, &journal_id)
            .map_err(|e| error(ErrorClass::UnavailableSession, e))?;

        let skill_dirs = crate::agent::skills::discovery_paths(&config).await;
        self.agent
            .prepare(&config, &skill_dirs)
            .await
            .map_err(|e| error(ErrorClass::InvalidConfiguration, e))?;
        let gateway = self.gateway().await?;
        let mut command = self.agent.command(&config, input.env.as_ref(), &gateway);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);
        let mut child = command.spawn().map_err(|e| {
            error(
                ErrorClass::InvalidConfiguration,
                format!("Could not start the ACP agent: {e}"),
            )
        })?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            return Err(error(ErrorClass::Internal, "ACP agent stdio was not piped"));
        };
        self.live = Some(Live {
            child,
            rpc: Rpc::spawn(stdin, stdout),
            acp_session: String::new(),
            journal,
            model: None,
            mode: None,
            env_keys: env_keys(input),
            seed: None,
        });

        self.call(
            "initialize",
            json!({"protocolVersion": 1, "clientCapabilities": {}}),
            true,
        )
        .await?;
        let cwd = config.workspace_dir.clone();
        let meta = self.agent.session_meta(input.system_prompt.as_deref());
        let mut bound = bound_session(&existing, self.agent.name());
        if let Some(session) = &bound {
            if !self.agent.has_session(&config, session).await {
                bound = None;
            }
        }
        let loaded = match bound {
            Some(session) => self
                .call(
                    "session/load",
                    json!({"sessionId": session, "cwd": cwd, "mcpServers": [], "_meta": meta}),
                    true,
                )
                .await
                .map(|_| session)
                .inspect_err(|e| tracing::warn!(error = %e, "ACP session/load failed; reseeding a fresh session")),
            None => Err(error(ErrorClass::UnavailableSession, "unbound")),
        };
        let session = if let Ok(session) = loaded {
            session
        } else {
            {
                let created = self
                    .call(
                        "session/new",
                        json!({"cwd": cwd, "mcpServers": [], "_meta": meta}),
                        true,
                    )
                    .await?;
                let session = created["sessionId"]
                    .as_str()
                    .ok_or_else(|| {
                        error(
                            ErrorClass::NativeProtocol,
                            "session/new returned no sessionId",
                        )
                    })?
                    .to_string();
                if let Some(live) = self.live.as_mut() {
                    live.seed = seed(&existing);
                    live.journal.append(&json!({
                        "type": "binding", "agent": self.agent.name(),
                        "journal": journal_id, "acp_session": session,
                    }));
                }
                session
            }
        };
        if let Some(live) = &mut self.live {
            live.acp_session = session;
        }
        if let Some(t) = &mut self.trajectory {
            t.record(&journal_id);
        }
        Ok(())
    }

    /// Applies `input`'s model and mode to the idle session.
    async fn configure(&mut self, input: &Input) -> Result<(), HarnessError> {
        let Some(live) = &self.live else {
            return Ok(());
        };
        let session = live.acp_session.clone();
        if input.model.is_some() && input.model != live.model {
            self.call(
                "session/set_config_option",
                json!({"sessionId": session, "configId": "model", "value": input.model}),
                false,
            )
            .await
            .map_err(|mut e| {
                e.class = ErrorClass::InvalidConfiguration;
                e
            })?;
            if let Some(live) = &mut self.live {
                live.model.clone_from(&input.model);
            }
            if let Some(gateway) = &self.gateway {
                gateway.select(input.model.as_deref());
            }
        }
        let mode = self.agent.mode_id(input.mode.as_deref()).to_string();
        if self
            .live
            .as_ref()
            .is_some_and(|l| l.mode.as_deref() != Some(mode.as_str()))
        {
            self.call(
                "session/set_config_option",
                json!({"sessionId": session, "configId": "mode", "value": mode}),
                false,
            )
            .await?;
            if let Some(live) = &mut self.live {
                live.mode = Some(mode);
            }
        }
        Ok(())
    }

    /// Kills the agent and every process it started, including tool children
    /// that left its process group.
    async fn kill(&mut self) {
        let Some(mut live) = self.live.take() else {
            return;
        };
        #[cfg(unix)]
        if let Some(pid) = live.child.id() {
            let tree =
                tokio::task::spawn_blocking(move || crate::pod_processes::descendants_of(pid))
                    .await
                    .unwrap_or_default();
            for pid in tree.into_iter().chain([pid]) {
                if let Ok(pid) = i32::try_from(pid) {
                    let _ = nix::sys::signal::kill(
                        nix::unistd::Pid::from_raw(pid),
                        nix::sys::signal::Signal::SIGKILL,
                    );
                }
            }
        }
        let _ = live.child.kill().await;
    }

    /// Ends the execution: closes its journal turn, then reports totals and
    /// the terminal once the trajectory has flushed.
    fn finish(&mut self, failure: Option<HarnessError>) {
        let Some(mut exec) = self.exec.take() else {
            return;
        };
        let model = self.live.as_ref().and_then(|l| l.model.clone());
        if let Some(live) = &mut self.live {
            flush_text(
                &mut live.journal,
                &mut exec,
                model.as_deref(),
                &mut self.pending,
            );
        }
        let failure = failure.or(exec.failure.take()).or_else(|| {
            self.agent.reported_failure(&exec.replied).map(|class| {
                error(
                    class,
                    format!("The model call failed: {}", exec.replied.trim()),
                )
            })
        });
        let t = &exec.totals;
        if let Some(live) = &mut self.live {
            live.journal.append(&json!({
                "type": "turn_end", "id": uuid::Uuid::new_v4().to_string(), "model": model,
                "failed": failure.is_some(),
                "usage": {"input_tokens": t.input, "output_tokens": t.output,
                          "cache_read_tokens": t.cache_read, "cache_write_tokens": t.cache_write},
            }));
        }
        let as_i64 = |n: u64| i64::try_from(n).unwrap_or(i64::MAX);
        let totals = RunTotals {
            input_tokens: as_i64(t.input),
            output_tokens: as_i64(t.output),
            cache_creation_input_tokens: as_i64(t.cache_write),
            cache_read_input_tokens: as_i64(t.cache_read),
            reasoning_output_tokens: as_i64(t.thought),
            total_cost_usd: None,
            num_turns: Some(t.prompts),
            duration_ms: None,
            duration_api_ms: None,
            model,
            model_usage: None,
        };
        let termination = failure.map_or(ProcessTermination::Completed, ProcessTermination::Failed);
        self.after_flush = Some(vec![
            ProcessEvent::RunTotals(totals),
            ProcessEvent::ExecutionEnded { termination },
        ]);
    }

    fn session_id(&self) -> SessionId {
        SessionId(self.live.as_ref().map_or_default(|l| l.journal.id.clone()))
    }

    /// Opens an execution the agent started on its own, such as a wake-up
    /// after a background command.
    fn ensure_execution(&mut self) {
        if self.exec.is_some() {
            return;
        }
        self.pending.push_back(ProcessEvent::InternalStart {
            session: self.session_id(),
        });
        self.exec = Some(Execution {
            internal: true,
            ..Execution::default()
        });
    }

    fn handle(&mut self, message: Incoming) {
        match message {
            Incoming::Response { id, result } => self.on_response(id, result),
            Incoming::Notification { method, params } if method == "session/update" => {
                self.on_update(&params["update"]);
            }
            Incoming::Notification { .. } => {}
            Incoming::Request { id, method, params } => {
                if let Some(live) = &self.live {
                    answer_request(&live.rpc, id, &method, &params);
                }
            }
        }
    }

    fn on_response(&mut self, id: u64, result: Result<Value, Value>) {
        let Some(exec) = &mut self.exec else {
            return;
        };
        if !exec.prompts.remove(&id) {
            return;
        }
        match result {
            Ok(reply) => {
                let u = &reply["usage"];
                let n = |k: &str| u[k].as_u64().unwrap_or(0);
                exec.totals.input += n("inputTokens");
                exec.totals.output += n("outputTokens");
                exec.totals.cache_read += n("cachedReadTokens");
                exec.totals.cache_write += n("cachedWriteTokens");
                exec.totals.thought += n("thoughtTokens");
            }
            Err(e) => {
                let message = e["message"].as_str().unwrap_or_default().to_string();
                exec.failure.get_or_insert(error(
                    classify(&message),
                    format!("ACP prompt failed: {message}"),
                ));
            }
        }
        if exec.rearm(Instant::now()) {
            self.finish(None);
        }
    }

    fn on_update(&mut self, update: &Value) {
        let kind = update["sessionUpdate"].as_str().unwrap_or_default();
        if matches!(
            kind,
            "available_commands_update"
                | "config_option_update"
                | "current_mode_update"
                | "session_info_update"
        ) {
            return;
        }
        if kind == "usage_update" {
            if let Some(used) = update["used"].as_i64() {
                self.pending
                    .push_back(ProcessEvent::ContextWindow(ContextWindow {
                        used_tokens: used,
                        max_tokens: update["size"].as_i64(),
                    }));
            }
            return;
        }
        self.ensure_execution();
        let session = self.session_id();
        let model = self.live.as_ref().and_then(|l| l.model.clone());
        let (Some(live), Some(exec)) = (&mut self.live, &mut self.exec) else {
            return;
        };
        let text = update["content"]["text"].as_str().unwrap_or_default();
        match kind {
            "agent_message_chunk" => {
                if !exec.thought.is_empty() {
                    flush_text(&mut live.journal, exec, model.as_deref(), &mut self.pending);
                }
                exec.reply.push_str(text);
                exec.replied.push_str(text);
            }
            "agent_thought_chunk" => {
                if !exec.reply.is_empty() {
                    flush_text(&mut live.journal, exec, model.as_deref(), &mut self.pending);
                }
                exec.thought.push_str(text);
            }
            "tool_call" => {
                flush_text(&mut live.journal, exec, model.as_deref(), &mut self.pending);
                let call = update["toolCallId"].as_str().unwrap_or_default();
                let name = update["title"]
                    .as_str()
                    .or_else(|| update["kind"].as_str())
                    .unwrap_or("tool");
                let input = update["rawInput"].clone();
                if !matches!(update["status"].as_str(), Some("completed" | "failed")) {
                    exec.running.insert(call.to_string());
                }
                live.journal.append(&json!({
                    "type": "tool_call", "id": uuid::Uuid::new_v4().to_string(),
                    "tool_call_id": call, "name": name, "input": input,
                }));
                self.pending.push_back(ProcessEvent::Block(Block::ToolUse {
                    id: call.into(),
                    name: name.into(),
                    input,
                }));
                self.pending.push_back(ProcessEvent::Activity(Activity {
                    session,
                    detail: name.into(),
                }));
            }
            "tool_call_update" => {
                let status = update["status"].as_str().unwrap_or_default();
                let call = update["toolCallId"].as_str().unwrap_or_default();
                if !matches!(status, "completed" | "failed") {
                    self.pending.push_back(ProcessEvent::Activity(Activity {
                        session,
                        detail: "tool running".into(),
                    }));
                    exec.rearm(Instant::now());
                    return;
                }
                exec.running.remove(call);
                let content = tool_output(update);
                let is_error = status == "failed";
                live.journal.append(&json!({
                    "type": "tool_result", "id": uuid::Uuid::new_v4().to_string(),
                    "tool_call_id": call, "content": content, "is_error": is_error,
                }));
                self.pending
                    .push_back(ProcessEvent::Block(Block::ToolResult {
                        tool_use_id: call.into(),
                        content: Some(content),
                        is_error: Some(is_error),
                    }));
            }
            _ => {}
        }
        exec.rearm(Instant::now());
    }

    fn deadline(&self) -> Option<Instant> {
        let exec = self.exec.as_ref()?;
        let settle = match (exec.quiet_until, exec.background_until) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        exec.kill_at.or(settle)
    }

    async fn on_deadline(&mut self) {
        let killing = self
            .exec
            .as_ref()
            .is_some_and(|e| e.kill_at.is_some_and(|at| at <= Instant::now()));
        if killing {
            tracing::warn!("ACP agent did not answer session/cancel; killing its process tree");
            self.finish(None);
            self.kill().await;
        } else {
            self.finish(None);
        }
    }
}

/// Names of the env the agent was spawned with.
fn env_keys(input: &Input) -> BTreeSet<String> {
    input
        .env
        .iter()
        .flat_map(|env| env.keys().cloned())
        .collect()
}

/// Writes the buffered thought and reply as their journal lines and blocks.
fn flush_text(
    journal: &mut Journal,
    exec: &mut Execution,
    model: Option<&str>,
    pending: &mut VecDeque<ProcessEvent>,
) {
    let thought = std::mem::take(&mut exec.thought);
    if !thought.is_empty() {
        journal.append(
            &json!({"type": "thought", "id": uuid::Uuid::new_v4().to_string(), "text": thought}),
        );
        pending.push_back(ProcessEvent::Block(Block::Thinking {
            thinking: thought,
            signature: String::new(),
        }));
    }
    let reply = std::mem::take(&mut exec.reply);
    if !reply.is_empty() {
        journal.append(&json!({"type": "assistant", "id": uuid::Uuid::new_v4().to_string(), "model": model, "text": reply}));
        pending.push_back(ProcessEvent::Block(Block::Text { text: reply }));
    }
}

/// A tool call's output as one value: its text content joined, else its raw output.
fn tool_output(update: &Value) -> Value {
    let mut parts = Vec::new();
    for item in update["content"].as_array().into_iter().flatten() {
        match item["type"].as_str() {
            Some("content") => {
                if let Some(text) = item["content"]["text"].as_str() {
                    parts.push(text.to_string());
                }
            }
            Some("diff") => parts.push(format!(
                "Edited {}",
                item["path"].as_str().unwrap_or_default()
            )),
            _ => {}
        }
    }
    if parts.is_empty() {
        update["rawOutput"].clone()
    } else {
        Value::String(parts.join("\n"))
    }
}

/// Answers a request the agent sent: permission is granted, anything else is
/// refused as unsupported, since the client advertised no capabilities.
fn answer_request(rpc: &Rpc, id: Value, method: &str, params: &Value) {
    if method != "session/request_permission" {
        rpc.respond_error(id, -32601, "method not supported by this client");
        return;
    }
    let options = params["options"].as_array().cloned().unwrap_or_default();
    let pick = ["allow_always", "allow_once"]
        .iter()
        .find_map(|kind| options.iter().find(|o| o["kind"] == *kind))
        .or_else(|| options.first());
    match pick.and_then(|o| o["optionId"].as_str()) {
        Some(option) => rpc.respond(
            id,
            json!({"outcome": {"outcome": "selected", "optionId": option}}),
        ),
        None => rpc.respond(id, json!({"outcome": {"outcome": "cancelled"}})),
    }
}

fn classify(message: &str) -> ErrorClass {
    let lower = message.to_ascii_lowercase();
    if lower.contains("401") || lower.contains("403") || lower.contains("unauthorized") {
        ErrorClass::Authentication
    } else if lower.contains("429") || lower.contains("rate limit") {
        ErrorClass::RateLimit
    } else {
        ErrorClass::NativeProtocol
    }
}

#[async_trait::async_trait]
impl AgentProcess for AcpProcess {
    async fn submit(&mut self, input: Input) -> Result<(), HarnessError> {
        if task_token(&self.cfg.config).await.is_none() {
            return Err(error(
                ErrorClass::Authentication,
                "No task token for the model gateway",
            ));
        }
        let active = self.exec.is_some();
        if !self.reusable(&input) {
            if active {
                tracing::warn!(input = %input.id.0, "ACP agent refused an injection that needs a new session");
                return Err(error(
                    ErrorClass::InvalidInput,
                    "A new session or agent configuration needs a new turn",
                ));
            }
            self.kill().await;
            if let Err(mut e) = self.open(&input).await {
                e.stage = FailureStage::Startup;
                self.kill().await;
                return Err(e);
            }
        }
        if !active {
            if let Err(mut e) = self.configure(&input).await {
                e.stage = FailureStage::Startup;
                return Err(e);
            }
            self.pending.push_back(ProcessEvent::SessionReady {
                session: self.session_id(),
                agent_turn_id: None,
            });
            self.exec = Some(Execution::default());
        }
        let (Some(live), Some(exec)) = (self.live.as_mut(), self.exec.as_mut()) else {
            return Err(error(ErrorClass::Internal, "ACP agent was not opened"));
        };
        exec.internal = false;
        exec.quiet_until = None;
        exec.background_until = None;
        live.journal
            .append(&json!({"type": "user", "id": input.id.0, "text": input.text}));
        let text = match live.seed.take() {
            Some(seed) => format!("{seed}{}", input.text),
            None => input.text.clone(),
        };
        let id = live.rpc.request(
            "session/prompt",
            json!({"sessionId": live.acp_session, "prompt": [{"type": "text", "text": text}]}),
        );
        exec.prompts.insert(id);
        exec.totals.prompts += 1;
        self.pending
            .push_back(ProcessEvent::InputAccepted { input: input.id });
        Ok(())
    }

    fn accepts_injection(&self) -> bool {
        true
    }

    async fn cancel(&mut self) {
        let Some(exec) = &mut self.exec else {
            return;
        };
        if let Some(live) = &self.live {
            live.rpc
                .notify("session/cancel", json!({"sessionId": live.acp_session}));
        }
        if exec.prompts.is_empty() {
            self.finish(None);
        } else {
            exec.kill_at = Some(Instant::now() + CANCEL_GRACE);
        }
    }

    async fn next(&mut self) -> Option<ProcessEvent> {
        loop {
            if self.after_flush.is_some() {
                // Taken only after the flush: a `next` dropped mid-flush
                // flushes again rather than losing the terminal.
                if let Some(t) = &self.trajectory {
                    t.flush().await;
                }
                self.pending
                    .extend(self.after_flush.take().unwrap_or_default());
            }
            if let Some(event) = self.pending.pop_front() {
                return Some(event);
            }
            if let Some(message) = self.stash.pop_front() {
                self.handle(message);
                continue;
            }
            let deadline = self.deadline();
            let Some(live) = &mut self.live else {
                // Nothing runs until the next submit spawns the agent; the
                // driver drops this future when that submit arrives.
                std::future::pending::<()>().await;
                continue;
            };
            let sleep = async {
                match deadline {
                    Some(at) => tokio::time::sleep_until(at).await,
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                message = live.rpc.recv() => if let Some(message) = message {
                    self.handle(message);
                } else {
                    let exit = error(ErrorClass::ProcessExit, "The ACP agent exited");
                    self.finish(Some(exit));
                    self.kill().await;
                },
                () = sleep => self.on_deadline().await,
            }
        }
    }
}

impl Drop for AcpProcess {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.live.as_ref().and_then(|l| l.child.id()) {
            for pid in crate::pod_processes::descendants_of(pid) {
                if let Ok(pid) = i32::try_from(pid) {
                    let _ = nix::sys::signal::kill(
                        nix::unistd::Pid::from_raw(pid),
                        nix::sys::signal::Signal::SIGKILL,
                    );
                }
            }
        }
    }
}
