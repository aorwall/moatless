//! Integration tests over the persistence tier — DAOs and their handlers.
//! One of three `integration_*` test targets; shared fixtures live in
//! `tests/common`.

// Every module below except `supervisor_test` carries its own
// `#![cfg(feature = "db-tests")]`, so `common` is unreachable without it.
#[cfg(feature = "db-tests")]
#[path = "../common/mod.rs"]
mod common;

mod feedback_dao_test;
mod feedback_handler_test;
mod message_dao_test;
mod message_handler_test;
mod plugin_skills_dao_test;
mod repository_dao_test;
mod repository_handler_test;
mod supervisor_test;
mod t3_usage_port_test;
mod tag_dao_test;
mod tag_handler_test;
mod task_bindings_port_test;
mod task_dao_test;
mod task_event_dao_test;
mod task_handler_test;
mod task_listing_position_test;
mod task_turn_dao_test;
mod task_workspace_test;
mod user_uuid_test;
mod workspace_dao_test;
mod workspace_git_sync_test;
