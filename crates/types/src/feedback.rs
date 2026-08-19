//! Feedback vocabulary: who submitted a note, and about what.

crate::string_enum!(
    /// Submitter of a feedback row — derived from auth context on the server.
    FeedbackSource {
        /// Submitted by a regular user session.
        User => "user",
        /// Submitted by the agent via the sandbox-issued user token.
        Agent => "agent",
    }
);

crate::string_enum!(
    /// Where in the UI/CLI the feedback originated.
    FeedbackSurface {
        Task => "task",
        Message => "message",
        ToolCall => "tool_call",
        PreviewFailure => "preview_failure",
    }
);
