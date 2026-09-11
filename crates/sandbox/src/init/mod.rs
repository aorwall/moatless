//! Health stays unready through core initialization; workspace setup follows
//! readiness and reports failures without failing the process.

mod claude_code_settings;
pub mod codex_config;
mod dirs;
mod error;
pub(crate) mod git;
pub(crate) mod home_skills;
mod initialization;
mod models;
pub(crate) mod plugin_skills;
#[cfg(test)]
mod replacement_test;
mod workspace;

pub use error::InitError;
pub(crate) use initialization::initialize;
pub use models::InitPhase;
#[cfg(test)]
pub(crate) use crate::agent::restart::{
    WorkspaceOrigin, open_for_work, recover_interrupted_run_unless_pending,
};
