//! Projection of Moatless rows onto the shapes T3's shell contract expects.
//!
//! Everything T3 calls a field name lives here or in the transport above it.
//! Nothing below this module knows the vocabulary: the input is a `Workspace`
//! row, the output is `serde_json::Value` on the wire.

mod bounded;
mod compact;
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
mod v2;
mod v2_items;
mod v2_lifecycle;
mod v2_provider;
mod v2_subagents;

#[cfg(test)]
mod tests;

pub(crate) use bounded::HistoryCursor;
pub(crate) use bounded::bound_projection;
pub(crate) use bounded::history_page;
pub(crate) use compact::Compactions;
pub(crate) use compact::compact_snapshot;
pub(crate) use compact::with_id_table;
pub(crate) use detail::subtask;
pub(crate) use detail::thread_detail;
pub(crate) use events::change_events;
pub(crate) use events::is_session_change;
pub(crate) use events::turn_settled_events;
pub(crate) use origin::OriginContext;
pub(crate) use project::is_script_icon;
pub(crate) use project::project;
pub(crate) use project::ui_script;
pub(crate) use search::thread_matches;
pub(crate) use shell::listed_threads;
pub(crate) use shell::shell_snapshot;
pub(crate) use shell::thread_removed;
pub(crate) use shell::thread_shell;
pub(crate) use shell::thread_upserted;
pub(crate) use v2::Lineage;
pub(crate) use v2::archived_shell_snapshot_v2;
pub(crate) use v2::listed_threads_v2;
pub(crate) use v2::queued_message_uuid;
pub(crate) use v2::shell_snapshot_v2;
pub(crate) use v2::thread_projection;
pub(crate) use v2::thread_removed_v2;
pub(crate) use v2::thread_shell_v2;
pub(crate) use v2::thread_updated_v2;
pub(crate) use v2::turn_count_of_checkpoint;
pub(crate) use v2::turn_number_of_run;
