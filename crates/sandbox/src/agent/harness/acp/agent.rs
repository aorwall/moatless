//! What differs between ACP agents: how each is launched, configured and
//! where it keeps its state.
use std::path::PathBuf;

use moatless_types::agent::AgentProfile;
use serde_json::{Value, json};
use tokio::process::Command;

use crate::agent::harness::ErrorClass;
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

    /// The class of a model-call failure the agent reported as reply text
    /// rather than as an error: Antigravity ends the turn normally and writes
    /// `⚠ request failed … (code N)`.
    pub(super) fn reported_failure(self, reply: &str) -> Option<ErrorClass> {
        match self {
            Self::Antigravity => {
                // Only a reply that ends on the warning failed: one the agent
                // retried past carries it mid-text.
                let line = reply
                    .lines()
                    .rev()
                    .find(|l| !l.trim().is_empty())
                    .filter(|l| l.trim_start().starts_with("⚠ request failed"))?;
                let code = line
                    .rsplit_once("(code ")
                    .and_then(|(_, rest)| rest.split(')').next())
                    .and_then(|code| code.trim().parse::<u16>().ok());
                Some(match code {
                    Some(401 | 403) => ErrorClass::Authentication,
                    Some(429) => ErrorClass::RateLimit,
                    Some(500..) => ErrorClass::ProviderUnavailable,
                    Some(_) => ErrorClass::InvalidInput,
                    None => ErrorClass::Connectivity,
                })
            }
        }
    }
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

    #[test]
    fn a_failed_model_call_in_the_reply_is_classified_by_its_status() {
        let agent = AcpAgent::Antigravity;
        assert_eq!(
            agent.reported_failure("\n⚠ request failed with empty body (code 401)\n"),
            Some(ErrorClass::Authentication)
        );
        assert_eq!(
            agent.reported_failure("⚠ request failed: slow down (code 429)"),
            Some(ErrorClass::RateLimit)
        );
        assert_eq!(agent.reported_failure("All done."), None);
        assert_eq!(
            agent.reported_failure("⚠ request failed (code 503)\nRetried; all done."),
            None
        );
    }
}
