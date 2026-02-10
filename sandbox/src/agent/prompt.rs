//! Interactive prompt handling via oneshot channels.
//!
//! When the agent encounters an AskUserQuestion or ExitPlanMode tool,
//! it creates a oneshot channel and stores the sender in AgentState.
//! The message consumer (or an HTTP endpoint) resolves the prompt by
//! sending the response on the channel.

use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

/// The tool name that triggered the interactive prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PromptToolName {
    AskUserQuestion,
    ExitPlanMode,
}

impl PromptToolName {
    /// Parse from the tool name string sent by the Claude CLI.
    pub fn from_tool_name(name: &str) -> Option<Self> {
        match name {
            "AskUserQuestion" => Some(Self::AskUserQuestion),
            "ExitPlanMode" => Some(Self::ExitPlanMode),
            _ => None,
        }
    }
}

/// A pending prompt waiting for user input.
#[allow(dead_code)]
pub struct PendingPrompt {
    /// Which tool triggered the prompt.
    pub tool_name: PromptToolName,
    /// The tool input (questions, plan content, etc.).
    pub input: serde_json::Value,
    /// Sender to resolve the prompt.
    pub sender: oneshot::Sender<PromptResponse>,
}

/// Response to an interactive prompt.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PromptResponse {
    /// Answer to AskUserQuestion.
    #[serde(rename = "askUserAnswer")]
    AskUserAnswer { answers: serde_json::Value },
    /// Plan approved (ExitPlanMode).
    #[serde(rename = "planApproved")]
    PlanApproved,
    /// Plan rejected with feedback (ExitPlanMode).
    #[serde(rename = "planRejected")]
    PlanRejected { feedback: String },
}
