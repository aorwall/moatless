use std::{collections::VecDeque, process::Stdio, sync::Arc, time::Duration};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio_util::sync::CancellationToken;

use super::{
    MiniSweConfig,
    session::{MiniSweSessionFormat, blocks},
};
use crate::agent::harness::process::{AgentProcess, ProcessEvent, ProcessTermination};
use crate::agent::harness::signal::{Activity, Input, RunTotals, SessionId};
use crate::agent::harness::{ErrorClass, FailureStage, HarnessError, SessionImpact};
use crate::agent::types::ResumeSession;
use crate::trajectory_tail::TurnTrajectory;

pub(crate) struct MiniSweProcess {
    cfg: MiniSweConfig,
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    stdout: Option<BufReader<ChildStdout>>,
    line: Vec<u8>,
    pending: VecDeque<ProcessEvent>,
    inputs: VecDeque<super::super::InputId>,
    session: String,
    trajectory: Option<TurnTrajectory>,
    cancelled: CancellationToken,
    failure: Option<HarnessError>,
    totals: RunTotals,
    ended: bool,
}

fn error(class: ErrorClass, diagnostic: impl ToString) -> HarnessError {
    HarnessError::new(
        FailureStage::Execution,
        class,
        SessionImpact::Intact,
        diagnostic.to_string(),
    )
}

impl MiniSweProcess {
    pub(crate) fn new(cfg: MiniSweConfig) -> Self {
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
            .ok_or_else(|| error(ErrorClass::ProcessExit, "MiniSwe stdin closed"))?;
        let mut bytes = serde_json::to_vec(&value).map_err(|e| error(ErrorClass::Internal, e))?;
        bytes.push(b'\n');
        tokio::time::timeout(Duration::from_secs(10), stdin.write_all(&bytes))
            .await
            .map_err(|e| error(ErrorClass::UnresponsiveProcess, e))?
            .map_err(|e| error(ErrorClass::ProcessExit, e))
    }

    async fn open(&mut self, input: &Input) -> Result<(), HarnessError> {
        if input.mode.as_deref() == Some("plan") || input.effort.is_some() {
            return Err(error(
                ErrorClass::InvalidConfiguration,
                "mini-swe does not support plan mode or reasoning controls",
            ));
        }
        if matches!(input.resume, Some(ResumeSession::Last))
            && !input.start_fresh
            && input.fork.is_none()
            && !input.switch_harness
        {
            return Err(error(
                ErrorClass::UnavailableSession,
                "MiniSwe requires an explicit session id to resume",
            ));
        }
        let config = Arc::clone(&self.cfg.config);
        let home = config.mini_swe_home_dir();
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
            Arc::new(MiniSweSessionFormat),
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
                "MiniSwe could not restore the fork or switched conversation",
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
                "Invalid MiniSwe session id",
            ));
        }
        let session_path = home
            .join("sessions")
            .join(format!("{}.jsonl", self.session));
        if !tokio::fs::try_exists(&session_path).await.unwrap_or(false) {
            if resume.is_some() {
                return Err(error(
                    ErrorClass::UnavailableSession,
                    "MiniSwe session is missing",
                ));
            }
            let header = json!({"type":"session","version":1,"id":self.session,"timestamp":chrono::Utc::now().to_rfc3339(),"cwd":config.workspace_dir});
            tokio::fs::write(&session_path, format!("{header}\n"))
                .await
                .map_err(|e| error(ErrorClass::UnavailableSession, e))?;
        }
        let model = input.model.as_deref().ok_or_else(|| {
            error(
                ErrorClass::InvalidConfiguration,
                "MiniSwe requires provider/model",
            )
        })?;
        let (provider, _) = moatless_types::agent::split_vendored_model(
            moatless_types::agent::MINI_SWE_VENDORS,
            model,
        )
        .ok_or_else(|| {
            error(
                ErrorClass::InvalidConfiguration,
                "mini-swe-agent requires anthropic/model or openai/model",
            )
        })?;
        self.totals.model = Some(model.into());
        let base = config.backend_api_url.as_deref().ok_or_else(|| {
            error(
                ErrorClass::InvalidConfiguration,
                "mini-swe requires the model gateway",
            )
        })?;
        let api_base = format!(
            "{}/llm/{provider}{}",
            base.trim_end_matches('/'),
            if provider == "openai" { "/v1" } else { "" }
        );
        let mut command = Command::new("mini-swe-python");
        if let Some(env) = &input.env {
            command.envs(env);
        }
        for key in [
            "ANTHROPIC_AUTH_TOKEN",
            "CLAUDE_CODE_OAUTH_TOKEN",
            "CODEX_AUTH_JSON",
            "ANTHROPIC_API_KEY",
            "OPENAI_API_KEY",
            "OPENAI_BASE_URL",
            "ANTHROPIC_BASE_URL",
        ] {
            command.env_remove(key);
        }
        command
            .env(
                "MSWEA_GLOBAL_CONFIG_DIR",
                std::path::Path::new(&config.runtime_dir).join("mini-swe-config"),
            )
            .env("MSWEA_SILENT_STARTUP", "1")
            .current_dir(&config.workspace_dir)
            .args(["-u", "-c", include_str!("bridge.py")])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);
        let mut child = command
            .spawn()
            .map_err(|e| error(ErrorClass::InvalidConfiguration, e))?;
        self.stdin = child.stdin.take();
        self.stdout = child.stdout.take().map(BufReader::new);
        self.child = Some(child);
        let prompt = format!(
            "{}\nUse the bash tool for actions. Every response must include a bash call. To finish, execute a command whose first output line is COMPLETE_TASK_AND_SUBMIT_FINAL_OUTPUT and put your final answer on the following lines. Each command runs in a fresh shell.\n{}",
            input
                .system_prompt
                .as_deref()
                .unwrap_or("You are a coding agent."),
            crate::agent::skills::catalog_prompt(&config).await
        );
        self.send(
            json!({"journal":session_path,"session":self.session,"resume":resume.is_some(),
            "model":model,"api_base":api_base,"token_path":config.persisted_user_token_path(),
            "cwd":config.workspace_dir,"system_prompt":prompt}),
        )
        .await?;
        let cancelled = self.cancelled.clone();
        let response = tokio::select! {
            ()=cancelled.cancelled()=>return Err(error(ErrorClass::ProcessExit,"mini-swe startup cancelled")),
            result=tokio::time::timeout(Duration::from_secs(60),self.read())=>result.map_err(|e|error(ErrorClass::UnresponsiveProcess,e))??,
        };
        if response["type"] != "ready" || response["session"] != self.session {
            return Err(error(
                ErrorClass::NativeProtocol,
                "mini-swe did not confirm the requested session",
            ));
        }
        if let Some(t) = &mut self.trajectory {
            t.record(&self.session);
        }
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
            .ok_or_else(|| error(ErrorClass::ProcessExit, "MiniSwe stdout closed"))?;
        loop {
            let bytes = stdout
                .fill_buf()
                .await
                .map_err(|e| error(ErrorClass::ProcessExit, e))?;
            if bytes.is_empty() {
                return Err(error(
                    ErrorClass::ProcessExit,
                    "mini-swe exited before reporting completion",
                ));
            }
            let end = bytes.iter().position(|b| *b == b'\n');
            let n = end.map_or(bytes.len(), |i| i + 1);
            if self.line.len() + n > 8 * 1024 * 1024 {
                return Err(error(
                    ErrorClass::NativeProtocol,
                    "mini-swe protocol line exceeds 8 MiB",
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
        if event["type"] == "input_accepted" {
            match self.inputs.pop_front() {
                Some(input) if event["id"].as_str() == Some(input.0.as_str()) => {
                    self.pending
                        .push_back(ProcessEvent::InputAccepted { input });
                }
                _ => {
                    self.failure = Some(error(
                        ErrorClass::NativeProtocol,
                        "Unexpected input acknowledgement",
                    ));
                }
            }
        }
        if event["type"] == "message" {
            let message = &event["record"]["message"];
            let usage = super::session::usage(message);
            self.totals.input_tokens += usage.input_tokens as i64;
            self.totals.output_tokens += usage.output_tokens as i64;
            self.totals.cache_read_input_tokens += usage.cache_read_tokens as i64;
            self.totals.cache_creation_input_tokens += usage.cache_write_tokens as i64;
            if message["object"] == "response" || message["role"] == "assistant" {
                *self.totals.num_turns.get_or_insert(0) += 1;
            }
            *self.totals.total_cost_usd.get_or_insert(0.0) +=
                message["extra"]["cost"].as_f64().unwrap_or(0.0);
            self.pending
                .extend(blocks(message).into_iter().map(ProcessEvent::Block));
        }
        self.pending.push_back(ProcessEvent::Activity(Activity {
            session: SessionId(self.session.clone()),
            detail: event["type"].as_str().unwrap_or("mini-swe").into(),
        }));
    }
}

#[async_trait::async_trait]
impl AgentProcess for MiniSweProcess {
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
                "MiniSwe configuration changes require a new turn",
            ));
        }
        if self.inputs.len() >= 16 {
            return Err(error(
                ErrorClass::InvalidInput,
                "mini-swe steering queue is full",
            ));
        }
        self.send(json!({"id":input.id.0,"text":input.text}))
            .await?;
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
                Ok(event) if event["type"] == "finished" => {
                    self.close().await;
                    if event["exit_status"] != "Submitted" && self.failure.is_none() {
                        self.failure = Some(error(
                            match event["exit_status"].as_str() {
                                Some("AuthenticationError" | "PermissionDeniedError") => {
                                    ErrorClass::Authentication
                                }
                                Some("RateLimitError") => ErrorClass::RateLimit,
                                Some("APIConnectionError" | "Timeout") => ErrorClass::Connectivity,
                                Some("ContextWindowExceededError" | "BadRequestError") => {
                                    ErrorClass::InvalidInput
                                }
                                Some("RepeatedFormatError") => ErrorClass::NativeProtocol,
                                _ => ErrorClass::ProcessExit,
                            },
                            format!("mini-swe ended: {}", event["exit_status"]),
                        ));
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
                    if self.failure.is_some() {
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
