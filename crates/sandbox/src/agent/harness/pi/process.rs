use std::{collections::VecDeque, process::Stdio, sync::Arc, time::Duration};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio_util::sync::CancellationToken;

use super::{
    PiConfig,
    session::{PiSessionFormat, blocks},
};
use crate::agent::harness::process::{AgentProcess, ProcessEvent, ProcessTermination};
use crate::agent::harness::signal::{
    Activity, ContextWindow, Input, InputId, RunTotals, SessionId,
};
use crate::agent::harness::{ErrorClass, FailureStage, HarnessError, SessionImpact};
use crate::agent::types::ResumeSession;
use crate::trajectory_tail::TurnTrajectory;

#[cfg(test)]
mod tests;

pub(crate) struct PiProcess {
    cfg: PiConfig,
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    stdout: Option<BufReader<ChildStdout>>,
    line: Vec<u8>,
    pending: VecDeque<ProcessEvent>,
    inputs: VecDeque<InputId>,
    session: String,
    trajectory: Option<TurnTrajectory>,
    cancelled: CancellationToken,
    failure: Option<HarnessError>,
    totals: RunTotals,
    ended: bool,
    context_limit: Option<i64>,
}

fn error(class: ErrorClass, diagnostic: impl ToString) -> HarnessError {
    HarnessError::new(
        FailureStage::Execution,
        class,
        SessionImpact::Intact,
        diagnostic.to_string(),
    )
}

impl PiProcess {
    pub(crate) fn new(cfg: PiConfig) -> Self {
        Self {
            cfg,
            child: None,
            stdin: None,
            stdout: None,
            line: Vec::new(),
            pending: VecDeque::new(),
            inputs: VecDeque::new(),
            session: String::new(),
            trajectory: None,
            cancelled: CancellationToken::new(),
            failure: None,
            ended: false,
            context_limit: None,
            totals: RunTotals {
                input_tokens: 0,
                output_tokens: 0,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
                reasoning_output_tokens: 0,
                total_cost_usd: Some(0.0),
                num_turns: Some(0),
                duration_ms: None,
                duration_api_ms: None,
                model: None,
                model_usage: None,
            },
        }
    }

    async fn send(&mut self, value: Value) -> Result<(), HarnessError> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| error(ErrorClass::ProcessExit, "Pi stdin closed"))?;
        let mut bytes = serde_json::to_vec(&value).map_err(|e| error(ErrorClass::Internal, e))?;
        bytes.push(b'\n');
        tokio::time::timeout(Duration::from_secs(10), stdin.write_all(&bytes))
            .await
            .map_err(|e| error(ErrorClass::UnresponsiveProcess, e))?
            .map_err(|e| error(ErrorClass::ProcessExit, e))
    }

    async fn open(&mut self, input: &Input) -> Result<(), HarnessError> {
        if matches!(input.resume, Some(ResumeSession::Last))
            && !input.start_fresh
            && input.fork.is_none()
            && !input.switch_harness
        {
            return Err(error(
                ErrorClass::UnavailableSession,
                "Pi requires an explicit session id to resume",
            ));
        }
        let config = Arc::clone(&self.cfg.config);
        let home = std::path::Path::new(&config.runtime_dir).join("pi");
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
        tokio::fs::write(
            home.join("settings.json"),
            r#"{"steeringMode":"one-at-a-time"}"#,
        )
        .await
        .map_err(|e| error(ErrorClass::InvalidConfiguration, e))?;
        self.trajectory = TurnTrajectory::new(
            self.cfg.trajectory.as_ref(),
            Arc::new(PiSessionFormat),
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
                "Pi could not restore the fork or switched conversation",
            ));
        }
        let resume = restored.or_else(|| {
            if input.start_fresh {
                None
            } else {
                match &input.resume {
                    Some(ResumeSession::WithId(id)) => Some(id.clone()),
                    _ => None,
                }
            }
        });
        self.session = resume
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        if uuid::Uuid::parse_str(&self.session).is_err() {
            return Err(error(
                ErrorClass::UnavailableSession,
                "Invalid Pi session id",
            ));
        }
        let session_path = home
            .join("sessions")
            .join(format!("{}.jsonl", self.session));
        if !tokio::fs::try_exists(&session_path).await.unwrap_or(false) {
            if resume.is_some() {
                return Err(error(
                    ErrorClass::UnavailableSession,
                    "Pi session is missing",
                ));
            }
            let header = json!({"type":"session","version":3,"id":self.session,"timestamp":chrono::Utc::now().to_rfc3339(),"cwd":config.workspace_dir});
            tokio::fs::write(&session_path, format!("{header}\n"))
                .await
                .map_err(|e| error(ErrorClass::UnavailableSession, e))?;
        }
        let model = input.model.as_deref().ok_or_else(|| {
            error(
                ErrorClass::InvalidConfiguration,
                "Pi requires provider/model",
            )
        })?;
        let (provider, name) = moatless_types::agent::split_pi_model(model).ok_or_else(|| {
            error(
                ErrorClass::InvalidConfiguration,
                "Pi supports anthropic/model, openai/model or openrouter/model",
            )
        })?;
        self.totals.model = Some(model.into());
        let mut command = Command::new("pi");
        if let Some(env) = &input.env {
            command.envs(env);
        }
        for key in [
            "ANTHROPIC_AUTH_TOKEN",
            "CLAUDE_CODE_OAUTH_TOKEN",
            "CODEX_AUTH_JSON",
            "PI_API_KEY",
        ] {
            command.env_remove(key);
        }
        if let Some(base) = &config.backend_api_url {
            let token_path = config.persisted_user_token_path();
            let helper = format!(
                "!cat '{}'",
                token_path.to_string_lossy().replace('\'', "'\\''")
            );
            let mut providers = serde_json::Map::new();
            for p in ["anthropic", "openai"] {
                let suffix = if p == "openai" { "/v1" } else { "" };
                providers.insert(p.into(),json!({"baseUrl":format!("{}/llm/{p}{suffix}",base.trim_end_matches('/')),"apiKey":helper,"authHeader":true,"headers":{"X-Moatless-Auth-Mode":"api-key-or-hub"}}));
            }
            if provider == "openrouter" {
                providers.insert(
                    "openrouter".into(),
                    openrouter_provider(base, &helper, name),
                );
            }
            tokio::fs::write(
                home.join("models.json"),
                json!({"providers":providers}).to_string(),
            )
            .await
            .map_err(|e| error(ErrorClass::InvalidConfiguration, e))?;
            command
                .env_remove("ANTHROPIC_API_KEY")
                .env_remove("OPENAI_API_KEY");
        }
        command
            .env("PI_CODING_AGENT_DIR", &home)
            .current_dir(&config.workspace_dir)
            .args([
                "--offline",
                "--no-approve",
                "--no-extensions",
                "--no-skills",
                "--no-prompt-templates",
                "--mode",
                "rpc",
                "--provider",
                provider,
                "--model",
                name,
                "--session",
            ])
            .arg(session_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        for path in crate::agent::skills::discovery_paths(&config).await {
            if tokio::fs::metadata(&path).await.is_ok_and(|m| m.is_dir()) {
                command.arg("--skill").arg(path);
            }
        }
        if let Some(prompt) = &input.system_prompt {
            command.arg("--append-system-prompt").arg(prompt);
        }
        if let Some(effort) = &input.effort {
            command.arg("--thinking").arg(effort);
        }
        #[cfg(unix)]
        command.process_group(0);
        let mut child = command
            .spawn()
            .map_err(|e| error(ErrorClass::InvalidConfiguration, e))?;
        self.stdin = child.stdin.take();
        self.stdout = child.stdout.take().map(BufReader::new);
        self.child = Some(child);
        self.send(json!({"type":"get_state","id":"state"})).await?;
        let cancelled = self.cancelled.clone();
        let response = tokio::select! {
            ()=cancelled.cancelled()=>return Err(error(ErrorClass::ProcessExit,"Pi startup cancelled")),
            result=tokio::time::timeout(Duration::from_secs(30),self.read())=>result.map_err(|e|error(ErrorClass::UnresponsiveProcess,e))??,
        };
        if response["id"] != "state"
            || response["success"] != true
            || response["data"]["sessionId"] != self.session
        {
            return Err(error(
                ErrorClass::NativeProtocol,
                "Pi did not confirm the requested session",
            ));
        }
        if let Some(t) = &mut self.trajectory {
            t.record(&self.session);
        }
        self.context_limit = response["data"]["model"]["contextWindow"].as_i64();
        self.pending.push_back(ProcessEvent::SessionReady {
            session: SessionId(self.session.clone()),
            agent_turn_id: None,
        });
        Ok(())
    }

    async fn read(&mut self) -> Result<Value, HarnessError> {
        let stdout = self
            .stdout
            .as_mut()
            .ok_or_else(|| error(ErrorClass::ProcessExit, "Pi stdout closed"))?;
        loop {
            let bytes = stdout
                .fill_buf()
                .await
                .map_err(|e| error(ErrorClass::ProcessExit, e))?;
            if bytes.is_empty() {
                return Err(error(
                    ErrorClass::ProcessExit,
                    "Pi exited before agent_settled",
                ));
            }
            let end = bytes.iter().position(|b| *b == b'\n');
            let n = end.map_or(bytes.len(), |i| i + 1);
            if self.line.len() + n > 8 * 1024 * 1024 {
                return Err(error(
                    ErrorClass::NativeProtocol,
                    "Pi RPC line exceeds 8 MiB",
                ));
            }
            self.line.extend_from_slice(&bytes[..n]);
            stdout.consume(n);
            if end.is_some() {
                let value = serde_json::from_slice(&self.line)
                    .map_err(|e| error(ErrorClass::NativeProtocol, e));
                self.line.clear();
                return value;
            }
        }
    }

    async fn close(&mut self) {
        self.stdin.take();
        if let Some(child) = &mut self.child {
            #[cfg(unix)]
            if let Some(pid) = child.id().and_then(|pid| i32::try_from(pid).ok()) {
                let _ = nix::sys::signal::killpg(
                    nix::unistd::Pid::from_raw(pid),
                    nix::sys::signal::Signal::SIGTERM,
                );
            }
            if tokio::time::timeout(Duration::from_secs(3), child.wait())
                .await
                .is_err()
            {
                let _ = child.kill().await;
                let _ = child.wait().await;
            }
        }
        if let Some(t) = &self.trajectory {
            t.flush().await;
        }
        self.ended = true;
    }

    fn observe(&mut self, event: &Value) {
        match event["type"].as_str() {
            Some("response") if event["success"] == false => {
                self.failure = Some(error(
                    ErrorClass::InvalidInput,
                    event["error"].as_str().unwrap_or("Pi rejected RPC command"),
                ));
            }
            Some("message_start") if event["message"]["role"] == "user" => {
                if let Some(input) = self.inputs.pop_front() {
                    self.pending
                        .push_back(ProcessEvent::InputAccepted { input });
                }
            }
            Some("message_end") => {
                let message = &event["message"];
                if message["role"] == "assistant" {
                    if message["stopReason"] == "error" {
                        self.failure = Some(super::error::provider_error(
                            message["errorMessage"]
                                .as_str()
                                .unwrap_or("Pi provider error"),
                        ));
                    }
                    let u = &message["usage"];
                    self.pending
                        .push_back(ProcessEvent::ContextWindow(ContextWindow {
                            used_tokens: u["input"].as_i64().unwrap_or(0)
                                + u["cacheRead"].as_i64().unwrap_or(0)
                                + u["cacheWrite"].as_i64().unwrap_or(0)
                                + u["output"].as_i64().unwrap_or(0),
                            max_tokens: self.context_limit,
                        }));
                    self.totals.input_tokens += u["input"].as_i64().unwrap_or(0);
                    self.totals.output_tokens += u["output"].as_i64().unwrap_or(0);
                    self.totals.cache_read_input_tokens += u["cacheRead"].as_i64().unwrap_or(0);
                    self.totals.cache_creation_input_tokens +=
                        u["cacheWrite"].as_i64().unwrap_or(0);
                    *self.totals.total_cost_usd.get_or_insert(0.0) +=
                        u["cost"]["total"].as_f64().unwrap_or(0.0);
                    *self.totals.num_turns.get_or_insert(0) += 1;
                }
                self.pending
                    .extend(blocks(message).into_iter().map(ProcessEvent::Block));
            }
            _ => {}
        }
        self.pending.push_back(ProcessEvent::Activity(Activity {
            session: SessionId(self.session.clone()),
            detail: event["type"].as_str().unwrap_or("pi").into(),
        }));
    }
}

#[async_trait::async_trait]
impl AgentProcess for PiProcess {
    async fn submit(&mut self, input: Input) -> Result<(), HarnessError> {
        if self.child.is_none() {
            if let Err(mut e) = self.open(&input).await {
                e.stage = FailureStage::Startup;
                self.close().await;
                return Err(e);
            }
        } else if input.start_fresh
            || input.fork.is_some()
            || input.switch_harness
            || input.model != self.totals.model
        {
            return Err(error(
                ErrorClass::InvalidInput,
                "Pi configuration changes require a new turn",
            ));
        } else if self.inputs.len() >= 16 {
            return Err(error(ErrorClass::InvalidInput, "Pi steering queue is full"));
        }
        self.send(json!({"type":"prompt","id":input.id.0,"message":input.text,"streamingBehavior":"steer"})).await?;
        self.inputs.push_back(input.id);
        Ok(())
    }
    fn accepts_injection(&self) -> bool {
        true
    }
    fn cancel_handle(&self) -> CancellationToken {
        self.cancelled.clone()
    }
    async fn cancel(&mut self) {
        let _ = self.send(json!({"type":"clear_queue","id":"clear"})).await;
        let _ = self.send(json!({"type":"abort","id":"abort"})).await;
        self.close().await;
        self.pending.clear();
        self.pending.push_back(ProcessEvent::Terminated {
            termination: ProcessTermination::Completed,
        });
    }
    async fn next(&mut self) -> Option<ProcessEvent> {
        loop {
            if let Some(e) = self.pending.pop_front() {
                return Some(e);
            }
            if self.ended {
                return None;
            }
            match self.read().await {
                Ok(event) if event["type"] == "agent_settled" => {
                    self.close().await;
                    if event["aborted"] == true && self.failure.is_none() {
                        self.failure = Some(error(ErrorClass::ProcessExit, "Pi aborted the run"));
                    }
                    let termination = self
                        .failure
                        .take()
                        .map_or(ProcessTermination::Completed, ProcessTermination::Failed);
                    self.pending
                        .push_back(ProcessEvent::RunTotals(self.totals.clone()));
                    self.pending
                        .push_back(ProcessEvent::Terminated { termination });
                }
                Ok(event) => {
                    self.observe(&event);
                    if self.failure.is_some() && event["type"] == "response" {
                        self.close().await;
                        self.pending.push_back(ProcessEvent::Terminated {
                            termination: ProcessTermination::Failed(self.failure.take()?),
                        });
                    }
                }
                Err(e) => {
                    self.close().await;
                    return Some(ProcessEvent::Terminated {
                        termination: ProcessTermination::Failed(e),
                    });
                }
            }
        }
    }
}

/// Pi's `openrouter` provider, pointed at the gateway's OpenAI route.
///
/// The hub serves `OpenRouter` over the Responses API, which is all the gateway
/// forwards; Pi's built-in `openrouter` speaks chat completions, a route the
/// gateway lacks. The model is declared here because the hub's ids are not all
/// in Pi's catalogue, and a declared model takes this provider's `api`.
fn openrouter_provider(base: &str, key_helper: &str, model: &str) -> serde_json::Value {
    json!({
        "baseUrl": format!("{}/llm/openai/v1", base.trim_end_matches('/')),
        "api": "openai-responses",
        "apiKey": key_helper,
        "authHeader": true,
        "headers": {"X-Moatless-Auth-Mode": "api-key-or-hub"},
        "models": [{"id": model, "reasoning": true}],
    })
}
