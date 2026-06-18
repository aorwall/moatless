//! Repo "loops" (agent automations) domain module.
//!
//! Reads `.moatless/loops/<name>/LOOP.md` from a repository's git tree and
//! reconciles each loop into the existing trigger tables (`triggers`,
//! `trigger_rules`, `*_trigger_sources`) tagged `managed_by = 'repo_loop'`.
//! Invoked from the repository create / sync-config handlers — this module
//! has no HTTP routes of its own.

pub mod error;
pub mod fetcher;
pub mod models;
pub mod parser;
pub mod sync;
