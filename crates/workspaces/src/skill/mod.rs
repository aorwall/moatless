//! Skill discovery domain module.
//!
//! Reads `.claude/skills/` directories from workspace and linked plugin
//! repositories, parses SKILL.md frontmatter, and merges the list the UI-RPC
//! skills surface serves.

pub mod error;
pub mod fetcher;
pub mod models;
pub mod scanner;
pub mod schemas;
pub mod service;
