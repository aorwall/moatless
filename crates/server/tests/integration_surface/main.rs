//! Integration tests over the client surface — `ui_rpc`, agent handlers,
//! pairing. One of three `integration_*` test targets; shared fixtures live in
//! `tests/common`.

// Every module below carries its own `#![cfg(feature = "db-tests")]`,
// so `common` is unreachable without it.
#[cfg(feature = "db-tests")]
#[path = "../common/mod.rs"]
mod common;

mod auth_operations_test;
mod closed_terminal_test;
mod event_subscriptions_test;
mod stranded_inbox_test;
mod task_switch_test;
mod ui_rpc_command_test;
mod ui_rpc_shell_test;
