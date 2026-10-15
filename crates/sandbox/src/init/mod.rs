//! Health stays unready through core initialization; workspace setup follows
//! readiness and reports failures without failing the process.

mod claude_code_settings;
pub mod codex_config;
mod dirs;
mod error;
pub(crate) mod git;
mod git_remotes;
pub(crate) mod home_skills;
mod initialization;
pub(crate) mod language_servers;
mod models;
pub(crate) mod plugin_skills;
#[cfg(test)]
mod replacement_test;
mod setup_gate;
mod workspace;

/// Re-exported beside [`InitPhase`]: both are init outcomes `SandboxState`
/// carries, and `agent::restart` is not public.
pub use crate::agent::restart::WorkspaceOrigin;
#[cfg(test)]
pub(crate) use crate::agent::restart::{open_for_work, recover_interrupted_run_unless_pending};
pub use error::InitError;
pub(crate) use initialization::initialize;
pub use models::InitPhase;
pub use setup_gate::SetupGate;
