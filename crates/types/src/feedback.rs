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

crate::string_enum!(
    /// What the note is about — the axis a reader triages on, orthogonal to
    /// [`FeedbackSurface`], which says only where it was submitted from. A
    /// row predating the column, or one whose submitter named none, carries
    /// no category.
    FeedbackCategory {
        /// The sandbox itself: a missing dependency or service, a command
        /// that cannot run here.
        Environment => "environment",
        /// The repository's own code or tooling — something in it that could
        /// be fixed or improved so this kind of work goes better.
        Codebase => "codebase",
        /// Instructions that are missing, wrong, or pointed the wrong way —
        /// `AGENTS.md`, `CLAUDE.md`, or a skill, including a skill that
        /// should exist and does not.
        Instruction => "instruction",
        /// Anything the other categories do not cover.
        Other => "other",
    }
);
