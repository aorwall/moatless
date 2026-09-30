//! Message vocabulary: the `user_type` values a Message row carries, and the
//! derived, UI-only events the live Chat renders.
//!
//! The events are assembled server-side and relayed to clients, never persisted
//! to `task_events`.

crate::string_enum!(
    /// Message sub-types — derived, UI-only events for the live Chat.
    ///
    /// `upsert` replaces-or-inserts an assembled Message by `uuid`;
    /// `subagentProgress` carries a subagent's absolute activity summary. Neither
    /// is an error.
    MessageSubType [Hash] {
        Upsert => "upsert",
        /// A subagent step advanced: carries the absolute `{ toolUseId,
        /// messageCount, tokens, status }` summary for one `Agent` tool call —
        /// keeps the card live without streaming transcript.
        ///
        /// Intentionally camelCase, not the crate's `snake_case` convention, so
        /// the wire value matches the frontend event contract.
        SubagentProgress => "subagentProgress",
    }
);

impl MessageSubType {
    pub fn is_error(&self) -> bool {
        false
    }
}

crate::string_enum!(
    /// Type of task message.
    MessageType {
        Assistant => "assistant",
        User => "user",
        System => "system",
        Result => "result",
        Error => "error",
    }
);

crate::string_enum!(
    /// Role of the message sender.
    MessageRole {
        User => "user",
        Assistant => "assistant",
        System => "system",
    }
);

/// `user_type` on a `user` row that exists only to hand a tool response to the
/// agent — the person's answer to `AskUserQuestion` or `ExitPlanMode`.
///
/// The answer renders under the call it settled, not as its own bubble; this
/// names the case explicitly rather than leaving readers to infer and drop the
/// row silently.
pub const TOOL_RESPONSE_USER_TYPE: &str = "tool_response";

#[cfg(test)]
mod wire_contract {
    use super::*;

    #[allow(clippy::unwrap_used, reason = "the assertion is the wire contract")]
    #[test]
    fn message_vocabulary_wire_strings() {
        for (value, wire) in [
            (MessageType::Assistant, "assistant"),
            (MessageType::User, "user"),
            (MessageType::System, "system"),
            (MessageType::Result, "result"),
            (MessageType::Error, "error"),
        ] {
            assert_eq!(value.as_str(), wire);
            assert_eq!(MessageType::try_from(wire), Ok(value));
            assert_eq!(
                serde_json::to_string(&value).unwrap(),
                format!("\"{wire}\"")
            );
        }
        for (value, wire) in [
            (MessageRole::User, "user"),
            (MessageRole::Assistant, "assistant"),
            (MessageRole::System, "system"),
        ] {
            assert_eq!(value.as_str(), wire);
            assert_eq!(MessageRole::try_from(wire), Ok(value));
            assert_eq!(
                serde_json::to_string(&value).unwrap(),
                format!("\"{wire}\"")
            );
        }
    }
}
