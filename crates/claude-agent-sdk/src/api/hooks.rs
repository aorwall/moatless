use serde::{Deserialize, Serialize};

/// All hook event types supported by the Claude CLI.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HookEvent {
    PreToolUse,
    PostToolUse,
    PostToolUseFailure,
    Notification,
    UserPromptSubmit,
    SessionStart,
    SessionEnd,
    Stop,
    SubagentStart,
    SubagentStop,
    PreCompact,
    PermissionRequest,
    Setup,
    #[serde(other)]
    Unknown,
}

impl std::fmt::Display for HookEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PreToolUse => write!(f, "PreToolUse"),
            Self::PostToolUse => write!(f, "PostToolUse"),
            Self::PostToolUseFailure => write!(f, "PostToolUseFailure"),
            Self::Notification => write!(f, "Notification"),
            Self::UserPromptSubmit => write!(f, "UserPromptSubmit"),
            Self::SessionStart => write!(f, "SessionStart"),
            Self::SessionEnd => write!(f, "SessionEnd"),
            Self::Stop => write!(f, "Stop"),
            Self::SubagentStart => write!(f, "SubagentStart"),
            Self::SubagentStop => write!(f, "SubagentStop"),
            Self::PreCompact => write!(f, "PreCompact"),
            Self::PermissionRequest => write!(f, "PermissionRequest"),
            Self::Setup => write!(f, "Setup"),
            Self::Unknown => write!(f, "Unknown"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_event_serializes_correctly() {
        assert_eq!(
            serde_json::to_string(&HookEvent::PreToolUse).unwrap(),
            "\"PreToolUse\""
        );
        assert_eq!(
            serde_json::to_string(&HookEvent::PostToolUse).unwrap(),
            "\"PostToolUse\""
        );
        assert_eq!(serde_json::to_string(&HookEvent::Stop).unwrap(), "\"Stop\"");
    }
}
