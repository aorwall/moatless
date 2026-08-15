//! Integration tests over the client surface — ui_rpc, agent handlers, pairing.
//! One of three `integration_*` test targets; shared fixtures live in
//! `tests/common`.

// Every module below carries its own `#![cfg(feature = "db-tests")]`,
// so `common` is unreachable without it.
#[cfg(feature = "db-tests")]
#[path = "../common/mod.rs"]
mod common;

mod agent_handlers_hooks_test;
mod agent_handlers_per_block_test;
mod closed_terminal_test;
mod device_pairing_test;
mod ui_rpc_command_test;
mod ui_rpc_preview_tabs_redis_test;
mod ui_rpc_shell_test;
mod ui_rpc_thread_test;
