//! What differs between ACP agents: how each is launched, configured and
//! where it keeps its state.
use std::path::PathBuf;

use moatless_types::agent::AgentProfile;
use serde_json::{Value, json};
use tokio::process::Command;

use chrono::Utc;

use crate::agent::harness::{ErrorClass, FailureStage, HarnessError, SessionImpact};
use crate::config::Config;

/// Credentials an ACP agent must never inherit: it reaches models only through
/// the Moatless gateway.
const STRIPPED_ENV: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "CODEX_AUTH_JSON",
    "OPENAI_API_KEY",
    "OPENAI_BASE_URL",
    "GEMINI_API_KEY",
    "GOOGLE_API_KEY",
    "GOOGLE_APPLICATION_CREDENTIALS",
];

/// An agent the ACP harness can run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AcpAgent {
    /// Google's Antigravity CLI, `agy acp`.
    Antigravity,
}

impl AcpAgent {
    pub(super) fn for_profile(profile: AgentProfile) -> Option<Self> {
        match profile {
            AgentProfile::Antigravity => Some(Self::Antigravity),
            _ => None,
        }
    }

    /// The agent's name on journal binding lines.
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Antigravity => "antigravity",
        }
    }

    fn state_dir(self, config: &Config) -> PathBuf {
        match self {
            Self::Antigravity => config.antigravity_dir(),
        }
    }

    /// Whether the agent still holds `session` on disk. Check before
    /// `session/load`: agy answers a load of an unknown id with an empty
    /// session and fails the next prompt with "trajectory not found".
    pub(super) async fn has_session(self, config: &Config, session: &str) -> bool {
        let path = match self {
            Self::Antigravity => self
                .state_dir(config)
                .join("antigravity-cli/conversations")
                .join(format!("{session}.db")),
        };
        tokio::fs::try_exists(path).await.unwrap_or(false)
    }

    /// Prepares the agent's state dir so it loads exactly `skill_dirs`.
    pub(super) async fn prepare(
        self,
        config: &Config,
        skill_dirs: &[PathBuf],
    ) -> std::io::Result<()> {
        let dir = self.state_dir(config).join("config");
        tokio::fs::create_dir_all(&dir).await?;
        let skills: Vec<Value> = skill_dirs.iter().map(|p| json!({ "path": p })).collect();
        tokio::fs::write(dir.join("skills.json"), Value::Array(skills).to_string()).await
    }

    /// The agent process, speaking ACP on stdio, with `env` layered over the
    /// inherited environment.
    pub(super) fn command(
        self,
        config: &Config,
        env: Option<&std::collections::HashMap<String, String>>,
        gateway_url: &str,
    ) -> Command {
        let mut command = match self {
            Self::Antigravity => {
                let mut command = Command::new("agy");
                command
                    .arg("acp")
                    .arg("--gemini-dir")
                    .arg(self.state_dir(config))
                    .args(["--dangerously-skip-permissions", "--disable-telemetry"]);
                command
            }
        };
        if let Some(env) = env {
            command.envs(env);
        }
        for key in STRIPPED_ENV {
            command.env_remove(key);
        }
        match self {
            Self::Antigravity => {
                command
                    .env("AGY_CLI_DISABLE_AUTO_UPDATE", "1")
                    .env("AGY_LLM_GATEWAY_URL", gateway_url)
                    .env("AGY_LLM_GATEWAY_API_KEY", super::gateway::PLACEHOLDER_KEY)
                    // The Gemini wire drops Claude's tool-use ids on the way back.
                    .env("AGY_LLM_GATEWAY_WIRE_PROTOCOL", "openai")
                    .env(
                        "AGY_LLM_GATEWAY_HEADERS",
                        "X-Moatless-Auth-Mode: api-key-or-hub",
                    );
            }
        }
        command.current_dir(&config.workspace_dir);
        command
    }

    /// `session/new` and `session/load` fields beyond the protocol's own.
    pub(super) fn session_meta(self, system_prompt: Option<&str>) -> Value {
        match (self, system_prompt) {
            (Self::Antigravity, Some(prompt)) => {
                json!({ "antigravity": { "systemPrompt": { "append": prompt } } })
            }
            (Self::Antigravity, None) => json!({}),
        }
    }

    /// The mode id for a Moatless mode.
    pub(super) fn mode_id(self, mode: Option<&str>) -> &'static str {
        match (self, mode) {
            (Self::Antigravity, Some("plan")) => "plan",
            (Self::Antigravity, _) => "default",
        }
    }

    /// A model-call failure the agent reported as reply text rather than as
    /// an error: Antigravity ends the turn normally and writes
    /// `⚠ request failed … (code N)` or, for a 429 with `Retry-After`,
    /// `⚠ The model provider asked to retry in N seconds`.
    pub(super) fn reported_failure(self, reply: &str) -> Option<HarnessError> {
        match self {
            Self::Antigravity => {
                // Only a reply that ends on the warning failed: one the agent
                // retried past carries it mid-text.
                let line = reply.lines().rev().find(|l| !l.trim().is_empty())?;
                let line = line.trim();
                let diagnostic = format!("The model call failed: {line}");
                if let Some(rest) = line.strip_prefix("⚠ The model provider asked to retry in ") {
                    let seconds = rest
                        .split_whitespace()
                        .next()
                        .and_then(|n| n.parse::<i64>().ok());
                    return Some(retry_after(seconds, &diagnostic));
                }
                if !line.starts_with("⚠ request failed") {
                    return None;
                }
                let code = line
                    .rsplit_once("(code ")
                    .and_then(|(_, rest)| rest.split(')').next())
                    .and_then(|code| code.trim().parse::<u16>().ok());
                let class = match code {
                    Some(401 | 403) => ErrorClass::Authentication,
                    Some(429) => ErrorClass::RateLimit,
                    Some(500..) => ErrorClass::ProviderUnavailable,
                    Some(_) => ErrorClass::InvalidInput,
                    None => ErrorClass::Connectivity,
                };
                Some(execution_error(class, diagnostic))
            }
        }
    }
}

/// A wait longer than the coordinator's longest backoff, which its retries
/// would only repeat: a usage limit, so a Loop falls back at once rather than
/// after five refused retries.
const USAGE_LIMIT_AFTER_SECS: i64 = 60;

/// The failure for a provider asking to be retried in `seconds`.
fn retry_after(seconds: Option<i64>, diagnostic: &str) -> HarnessError {
    match seconds {
        Some(seconds) if seconds > USAGE_LIMIT_AFTER_SECS => HarnessError::usage_limit(
            FailureStage::Execution,
            SessionImpact::Intact,
            diagnostic,
            Some(Utc::now() + chrono::Duration::seconds(seconds)),
        ),
        _ => execution_error(ErrorClass::ProviderThrottle, diagnostic),
    }
}

fn execution_error(class: ErrorClass, diagnostic: impl Into<String>) -> HarnessError {
    HarnessError::new(
        FailureStage::Execution,
        class,
        SessionImpact::Intact,
        diagnostic,
    )
}

/// The task token the gateway authenticates, re-read per call so a rotated
/// one takes effect at once.
pub(super) async fn task_token(config: &Config) -> Option<String> {
    match tokio::fs::read_to_string(config.persisted_user_token_path()).await {
        Ok(token) if !token.trim().is_empty() => Some(token.trim().to_string()),
        _ => config.user_auth_token.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn class(agent: AcpAgent, reply: &str) -> Option<ErrorClass> {
        agent.reported_failure(reply).map(|failure| failure.class)
    }

    #[test]
    fn a_failed_model_call_in_the_reply_is_classified_by_its_status() {
        let agent = AcpAgent::Antigravity;
        assert_eq!(
            class(agent, "\n⚠ request failed with empty body (code 401)\n"),
            Some(ErrorClass::Authentication)
        );
        assert_eq!(
            class(agent, "⚠ request failed: slow down (code 429)"),
            Some(ErrorClass::RateLimit)
        );
        assert_eq!(class(agent, "All done."), None);
        assert_eq!(
            class(agent, "⚠ request failed (code 503)\nRetried; all done."),
            None
        );
    }

    /// agy's words for a 429 carrying `Retry-After`, such as a hub credential
    /// in a quota cooldown.
    #[test]
    fn a_provider_asking_to_retry_is_a_usage_limit_with_its_reset_time() {
        let agent = AcpAgent::Antigravity;
        let before = chrono::Utc::now();
        let failure =
            agent.reported_failure("⚠ The model provider asked to retry in 5440 seconds\n");
        let failure = failure.expect("a retry request is a failure");
        assert_eq!(failure.class, ErrorClass::UsageLimit);
        let resets_at = failure.resets_at.expect("the seconds are a reset time");
        assert!(resets_at >= before + chrono::Duration::seconds(5440));
        assert!(resets_at <= chrono::Utc::now() + chrono::Duration::seconds(5440));

        let short = agent
            .reported_failure("⚠ The model provider asked to retry in 20 seconds")
            .expect("a retry request is a failure");
        assert_eq!(short.class, ErrorClass::ProviderThrottle);
        assert_eq!(short.resets_at, None);
    }
}
