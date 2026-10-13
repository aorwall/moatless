//! Interactive prompt handling via oneshot channels.
//!
//! When the agent encounters an `AskUserQuestion` or `ExitPlanMode` tool, it
//! creates a oneshot channel and stores the sender in `AgentState`. The message
//! consumer (or an HTTP endpoint) resolves the prompt by sending the response
//! on the channel.

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
    /// The call that parked the run. Carried so the answer can name the call it
    /// settles: the backend stores one row per tool-use id, and a panel closes
    /// on a row naming that id.
    pub tool_use_id: String,
    /// The tool input (questions, plan content, etc.).
    pub input: serde_json::Value,
    /// Sender to resolve the prompt.
    pub sender: oneshot::Sender<PromptResponse>,
}

/// Response to an interactive prompt.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PromptResponse {
    /// Answer to `AskUserQuestion`, one per question. Keyed on the question's
    /// own full text: that is the key the CLI looks an answer up by, and a
    /// question it finds no answer for is dropped from what the model is told.
    #[serde(rename = "askUserAnswer")]
    AskUserAnswer { answers: serde_json::Value },
    /// Freeform text answering `AskUserQuestion`, from someone who wrote
    /// instead of choosing.
    ///
    /// Needs no question to belong to, since the CLI tells the model "The user
    /// responded: …" instead of matching per question.
    #[serde(rename = "askUserResponse")]
    AskUserResponse { response: String },
    /// Plan approved (`ExitPlanMode`).
    #[serde(rename = "planApproved")]
    PlanApproved,
    /// Plan rejected with feedback (`ExitPlanMode`).
    #[serde(rename = "planRejected")]
    PlanRejected { feedback: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_tool_name_from_valid() {
        assert_eq!(
            PromptToolName::from_tool_name("AskUserQuestion"),
            Some(PromptToolName::AskUserQuestion)
        );
        assert_eq!(
            PromptToolName::from_tool_name("ExitPlanMode"),
            Some(PromptToolName::ExitPlanMode)
        );
    }

    #[test]
    fn prompt_tool_name_from_invalid() {
        assert_eq!(PromptToolName::from_tool_name("Write"), None);
        assert_eq!(PromptToolName::from_tool_name("Bash"), None);
        assert_eq!(PromptToolName::from_tool_name(""), None);
        assert_eq!(PromptToolName::from_tool_name("askuserquestion"), None);
    }

    #[test]
    fn prompt_tool_name_serializes_camel_case() {
        let json = serde_json::to_string(&PromptToolName::AskUserQuestion).unwrap();
        assert_eq!(json, r#""askUserQuestion""#);

        let json = serde_json::to_string(&PromptToolName::ExitPlanMode).unwrap();
        assert_eq!(json, r#""exitPlanMode""#);
    }

    #[test]
    fn prompt_response_deserializes_ask_user_answer() {
        let json = r#"{"type": "askUserAnswer", "answers": {"q1": "yes"}}"#;
        let resp: PromptResponse = serde_json::from_str(json).unwrap();
        match resp {
            PromptResponse::AskUserAnswer { answers } => {
                assert_eq!(answers["q1"], "yes");
            }
            _ => panic!("Expected AskUserAnswer"),
        }
    }

    #[test]
    fn prompt_response_deserializes_plan_approved() {
        let json = r#"{"type": "planApproved"}"#;
        let resp: PromptResponse = serde_json::from_str(json).unwrap();
        assert!(matches!(resp, PromptResponse::PlanApproved));
    }

    #[test]
    fn prompt_response_deserializes_plan_rejected() {
        let json = r#"{"type": "planRejected", "feedback": "needs more detail"}"#;
        let resp: PromptResponse = serde_json::from_str(json).unwrap();
        match resp {
            PromptResponse::PlanRejected { feedback } => {
                assert_eq!(feedback, "needs more detail");
            }
            _ => panic!("Expected PlanRejected"),
        }
    }

    #[test]
    fn pending_prompt_channel_drops_on_prompt_drop() {
        let (tx, rx) = oneshot::channel();
        let prompt = PendingPrompt {
            tool_name: PromptToolName::AskUserQuestion,
            tool_use_id: "toolu_test".to_string(),
            input: serde_json::json!({}),
            sender: tx,
        };

        drop(prompt);
        assert!(rx.blocking_recv().is_err());
    }
}
