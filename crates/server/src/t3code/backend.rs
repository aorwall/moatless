//! The Moatless side of the compatibility module's write ports.
//!
//! One implementation each of `ThreadCommandSink`, `BoundPullRequestSource`,
//! `IssueTrackerSource` and `AttachmentStore`, over `AppState`. Everything under here is
//! translation: a T3 command has already been validated by `super::commands`
//! by the time it arrives, and each method turns it into the same service
//! call the REST API makes.

mod attachments;
mod commands;
mod issues;
mod pull_request_reviews;
mod pull_requests;
#[cfg(test)]
mod tests;

pub use attachments::AppStateAttachments;
pub use commands::AppStateCommands;
pub use issues::LinearIssues;
pub use pull_requests::AppStateBoundPullRequests;
