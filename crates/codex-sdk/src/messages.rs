use codex_app_server_protocol::AgentMessageDeltaNotification;
use codex_app_server_protocol::CommandExecutionOutputDeltaNotification;
use codex_app_server_protocol::ConfigWarningNotification;
use codex_app_server_protocol::ContextCompactedNotification;
use codex_app_server_protocol::ErrorNotification;
use codex_app_server_protocol::FileChangeOutputDeltaNotification;
use codex_app_server_protocol::HookCompletedNotification;
use codex_app_server_protocol::HookStartedNotification;
use codex_app_server_protocol::ItemCompletedNotification;
use codex_app_server_protocol::ItemStartedNotification;
use codex_app_server_protocol::McpToolCallProgressNotification;
use codex_app_server_protocol::ModelReroutedNotification;
use codex_app_server_protocol::PlanDeltaNotification;
use codex_app_server_protocol::ReasoningSummaryTextDeltaNotification;
use codex_app_server_protocol::ReasoningTextDeltaNotification;
use codex_app_server_protocol::ServerNotification;
use codex_app_server_protocol::ThreadStartedNotification;
use codex_app_server_protocol::ThreadStatusChangedNotification;
use codex_app_server_protocol::ThreadTokenUsageUpdatedNotification;
use codex_app_server_protocol::TurnCompletedNotification;
use codex_app_server_protocol::TurnStartedNotification;

/// Messages streamed from the Codex agent.
///
/// This is the Codex equivalent of the Claude SDK's `SdkMessage`.
/// Each variant wraps the underlying protocol notification type directly,
/// avoiding unnecessary copies or projections.
#[derive(Debug, Clone)]
pub enum CodexMessage {
    // Thread lifecycle
    ThreadStarted(ThreadStartedNotification),
    ThreadStatusChanged(ThreadStatusChangedNotification),

    // Turn lifecycle
    TurnStarted(TurnStartedNotification),
    TurnCompleted(TurnCompletedNotification),

    // Content streaming
    AgentMessageDelta(AgentMessageDeltaNotification),
    ReasoningDelta(ReasoningTextDeltaNotification),
    ReasoningSummaryDelta(ReasoningSummaryTextDeltaNotification),

    // Items
    ItemStarted(ItemStartedNotification),
    ItemCompleted(ItemCompletedNotification),

    // Command execution output
    CommandOutputDelta(CommandExecutionOutputDeltaNotification),

    // File changes
    FileChangeOutputDelta(FileChangeOutputDeltaNotification),

    // Plan updates
    PlanDelta(PlanDeltaNotification),

    // MCP tool progress
    McpToolCallProgress(McpToolCallProgressNotification),

    // Hooks (observation only)
    HookStarted(HookStartedNotification),
    HookCompleted(HookCompletedNotification),

    // Token usage
    TokenUsageUpdated(ThreadTokenUsageUpdatedNotification),

    // Context compaction
    ContextCompacted(ContextCompactedNotification),

    // Model rerouting
    ModelRerouted(ModelReroutedNotification),

    // Errors
    Error(ErrorNotification),

    // Config warnings
    ConfigWarning(ConfigWarningNotification),

    // Forward-compatibility catch-all for notifications not yet mapped.
    Other(ServerNotification),
}

impl From<ServerNotification> for CodexMessage {
    fn from(notification: ServerNotification) -> Self {
        match notification {
            ServerNotification::ThreadStarted(n) => Self::ThreadStarted(n),
            ServerNotification::ThreadStatusChanged(n) => Self::ThreadStatusChanged(n),
            ServerNotification::TurnStarted(n) => Self::TurnStarted(n),
            ServerNotification::TurnCompleted(n) => Self::TurnCompleted(n),
            ServerNotification::AgentMessageDelta(n) => Self::AgentMessageDelta(n),
            ServerNotification::ReasoningTextDelta(n) => Self::ReasoningDelta(n),
            ServerNotification::ReasoningSummaryTextDelta(n) => Self::ReasoningSummaryDelta(n),
            ServerNotification::ItemStarted(n) => Self::ItemStarted(n),
            ServerNotification::ItemCompleted(n) => Self::ItemCompleted(n),
            ServerNotification::CommandExecutionOutputDelta(n) => Self::CommandOutputDelta(n),
            ServerNotification::FileChangeOutputDelta(n) => Self::FileChangeOutputDelta(n),
            ServerNotification::PlanDelta(n) => Self::PlanDelta(n),
            ServerNotification::McpToolCallProgress(n) => Self::McpToolCallProgress(n),
            ServerNotification::HookStarted(n) => Self::HookStarted(n),
            ServerNotification::HookCompleted(n) => Self::HookCompleted(n),
            ServerNotification::ThreadTokenUsageUpdated(n) => Self::TokenUsageUpdated(n),
            ServerNotification::ContextCompacted(n) => Self::ContextCompacted(n),
            ServerNotification::ModelRerouted(n) => Self::ModelRerouted(n),
            ServerNotification::Error(n) => Self::Error(n),
            ServerNotification::ConfigWarning(n) => Self::ConfigWarning(n),
            other => Self::Other(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_agent_message_delta() {
        let n = AgentMessageDeltaNotification {
            thread_id: "t1".into(),
            turn_id: "turn1".into(),
            item_id: "item1".into(),
            delta: "Hello world".into(),
        };
        let msg = CodexMessage::from(ServerNotification::AgentMessageDelta(n));
        match msg {
            CodexMessage::AgentMessageDelta(d) => {
                assert_eq!(d.delta, "Hello world");
                assert_eq!(d.thread_id, "t1");
            }
            other => panic!("Expected AgentMessageDelta, got {other:?}"),
        }
    }

    #[test]
    fn test_from_config_warning() {
        let n = ConfigWarningNotification {
            summary: "bad config".into(),
            details: Some("details here".into()),
            path: None,
            range: None,
        };
        let msg = CodexMessage::from(ServerNotification::ConfigWarning(n));
        match msg {
            CodexMessage::ConfigWarning(w) => {
                assert_eq!(w.summary, "bad config");
                assert_eq!(w.details.as_deref(), Some("details here"));
            }
            other => panic!("Expected ConfigWarning, got {other:?}"),
        }
    }

    #[test]
    fn test_from_thread_status_changed() {
        let n: ThreadStatusChangedNotification = serde_json::from_value(serde_json::json!({
            "threadId": "t1",
            "status": { "type": "idle" },
        }))
        .unwrap();
        let msg = CodexMessage::from(ServerNotification::ThreadStatusChanged(n));
        assert!(
            matches!(msg, CodexMessage::ThreadStatusChanged(_)),
            "got {msg:?}"
        );
    }

    #[test]
    fn test_from_thread_started() {
        let n: ThreadStartedNotification = serde_json::from_value(serde_json::json!({
            "thread": {
                "id": "t1",
                "preview": "hello",
                "ephemeral": true,
                "modelProvider": "openai",
                "createdAt": 1000,
                "updatedAt": 1000,
                "status": { "type": "idle" },
                "cwd": "/tmp",
                "cliVersion": "0.1.0",
                "source": "cli",
                "turns": [],
            }
        }))
        .unwrap();
        let msg = CodexMessage::from(ServerNotification::ThreadStarted(n));
        match msg {
            CodexMessage::ThreadStarted(ts) => {
                assert_eq!(ts.thread.id, "t1");
                assert!(ts.thread.ephemeral);
            }
            other => panic!("Expected ThreadStarted, got {other:?}"),
        }
    }

    #[test]
    fn test_from_turn_started() {
        let n: TurnStartedNotification = serde_json::from_value(serde_json::json!({
            "threadId": "t1",
            "turn": {
                "id": "turn1",
                "items": [],
                "status": "inProgress",
            }
        }))
        .unwrap();
        let msg = CodexMessage::from(ServerNotification::TurnStarted(n));
        assert!(matches!(msg, CodexMessage::TurnStarted(_)), "got {msg:?}");
    }

    #[test]
    fn test_from_turn_completed() {
        let n: TurnCompletedNotification = serde_json::from_value(serde_json::json!({
            "threadId": "t1",
            "turn": {
                "id": "turn1",
                "items": [],
                "status": "completed",
            }
        }))
        .unwrap();
        let msg = CodexMessage::from(ServerNotification::TurnCompleted(n));
        match msg {
            CodexMessage::TurnCompleted(tc) => {
                assert_eq!(tc.thread_id, "t1");
            }
            other => panic!("Expected TurnCompleted, got {other:?}"),
        }
    }

    #[test]
    fn test_from_error_notification() {
        let n: ErrorNotification = serde_json::from_value(serde_json::json!({
            "error": {
                "message": "something went wrong",
            },
            "willRetry": false,
            "threadId": "t1",
            "turnId": "turn1",
        }))
        .unwrap();
        let msg = CodexMessage::from(ServerNotification::Error(n));
        match msg {
            CodexMessage::Error(e) => {
                assert_eq!(e.error.message, "something went wrong");
                assert!(!e.will_retry);
            }
            other => panic!("Expected Error, got {other:?}"),
        }
    }

    #[test]
    fn test_from_command_output_delta() {
        let n = CommandExecutionOutputDeltaNotification {
            thread_id: "t1".into(),
            turn_id: "turn1".into(),
            item_id: "item1".into(),
            delta: "output line".into(),
        };
        let msg = CodexMessage::from(ServerNotification::CommandExecutionOutputDelta(n));
        match msg {
            CodexMessage::CommandOutputDelta(d) => {
                assert_eq!(d.delta, "output line");
            }
            other => panic!("Expected CommandOutputDelta, got {other:?}"),
        }
    }

    #[test]
    fn test_from_file_change_output_delta() {
        let n = FileChangeOutputDeltaNotification {
            thread_id: "t1".into(),
            turn_id: "turn1".into(),
            item_id: "item1".into(),
            delta: "diff content".into(),
        };
        let msg = CodexMessage::from(ServerNotification::FileChangeOutputDelta(n));
        assert!(
            matches!(msg, CodexMessage::FileChangeOutputDelta(_)),
            "got {msg:?}"
        );
    }

    #[test]
    fn test_from_context_compacted() {
        let n = ContextCompactedNotification {
            thread_id: "t1".into(),
            turn_id: "turn1".into(),
        };
        let msg = CodexMessage::from(ServerNotification::ContextCompacted(n));
        assert!(
            matches!(msg, CodexMessage::ContextCompacted(_)),
            "got {msg:?}"
        );
    }

    #[test]
    fn test_from_reasoning_delta() {
        let n: ReasoningTextDeltaNotification = serde_json::from_value(serde_json::json!({
            "threadId": "t1",
            "turnId": "turn1",
            "itemId": "item1",
            "delta": "thinking...",
            "contentIndex": 0,
        }))
        .unwrap();
        let msg = CodexMessage::from(ServerNotification::ReasoningTextDelta(n));
        match msg {
            CodexMessage::ReasoningDelta(r) => {
                assert_eq!(r.delta, "thinking...");
            }
            other => panic!("Expected ReasoningDelta, got {other:?}"),
        }
    }

    #[test]
    fn test_from_hook_started() {
        let n: HookStartedNotification = serde_json::from_value(serde_json::json!({
            "threadId": "t1",
            "turnId": "turn1",
            "run": {
                "id": "run1",
                "eventName": "preToolUse",
                "handlerType": "command",
                "executionMode": "sync",
                "scope": "turn",
                "sourcePath": "/tmp/hook.sh",
                "displayOrder": 0,
                "status": "running",
                "startedAt": 1000,
                "entries": [],
            }
        }))
        .unwrap();
        let msg = CodexMessage::from(ServerNotification::HookStarted(n));
        match msg {
            CodexMessage::HookStarted(h) => {
                assert_eq!(h.run.id, "run1");
            }
            other => panic!("Expected HookStarted, got {other:?}"),
        }
    }

    #[test]
    fn test_from_model_rerouted() {
        let n: ModelReroutedNotification = serde_json::from_value(serde_json::json!({
            "threadId": "t1",
            "turnId": "turn1",
            "fromModel": "gpt-4o",
            "toModel": "gpt-4o-mini",
            "reason": "highRiskCyberActivity",
        }))
        .unwrap();
        let msg = CodexMessage::from(ServerNotification::ModelRerouted(n));
        match msg {
            CodexMessage::ModelRerouted(r) => {
                assert_eq!(r.from_model, "gpt-4o");
                assert_eq!(r.to_model, "gpt-4o-mini");
            }
            other => panic!("Expected ModelRerouted, got {other:?}"),
        }
    }

    #[test]
    fn test_unknown_notification_maps_to_other() {
        // ThreadArchived is a valid ServerNotification not mapped by CodexMessage
        let n: codex_app_server_protocol::ThreadArchivedNotification =
            serde_json::from_value(serde_json::json!({
                "threadId": "t1",
            }))
            .unwrap();
        let msg = CodexMessage::from(ServerNotification::ThreadArchived(n));
        assert!(matches!(msg, CodexMessage::Other(_)), "got {msg:?}");
    }
}
