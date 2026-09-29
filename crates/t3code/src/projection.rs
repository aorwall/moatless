//! Projection of Moatless rows onto the shapes T3's shell contract expects.
//!
//! Everything T3 calls a field name lives here or in the transport above it.
//! Nothing below this module knows the vocabulary: the input is a `Workspace`
//! row, the output is `serde_json::Value` on the wire.

mod detail;
mod events;
mod model;
mod origin;
mod project;
mod pull_request;
mod scalar;
mod search;
mod session;
mod shell;
mod transcript;
mod turn;

#[cfg(test)]
mod tests;

pub(crate) use detail::subtask;
pub(crate) use detail::thread_detail;
pub(crate) use events::change_events;
pub(crate) use events::is_session_change;
pub(crate) use events::turn_settled_events;
pub(crate) use origin::OriginContext;
pub(crate) use project::is_script_icon;
pub(crate) use project::ui_script;
pub(crate) use search::thread_matches;
pub(crate) use shell::listed_threads;
pub(crate) use shell::shell_snapshot;
pub(crate) use shell::thread_removed;
pub(crate) use shell::thread_shell;
pub(crate) use shell::thread_upserted;
