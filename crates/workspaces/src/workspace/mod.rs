//! Workspace domain module.
//!
//! A Workspace owns the run-configuration and holds 0..n repository references
//! (`workspace_repos`). A Repository is a reusable git pointer; a Task
//! references a Workspace.

pub mod credentials;
pub(crate) mod dao;
pub mod error;
pub mod git_skills;
pub mod git_sync;
pub(crate) mod handlers;
pub mod models;
pub mod operations;
pub mod provision;
pub mod routes;
pub mod schemas;
pub mod scripts;

#[cfg(test)]
mod git_placements_test;

pub(crate) mod mutations;

pub(crate) mod config_validation;

pub use models::UpdateOutcome;
pub use operations::Workspaces;
