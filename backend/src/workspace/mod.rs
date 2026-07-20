//! Workspace domain module.
//!
//! A Workspace owns the run-configuration and holds 0..n repository references
//! (`workspace_repos`). A Repository is a reusable git pointer; a Task
//! references a Workspace. Introduced additively in Phase A — see
//! `docs/plans/workspace-concept.md`.

pub mod dao;
pub mod error;
pub mod handlers;
pub mod models;
pub mod routes;
pub mod schemas;
