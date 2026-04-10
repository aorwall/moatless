//! Skill discovery domain module.
//!
//! Reads `.claude/skills/` directories from workspace and linked plugin
//! repositories, parses SKILL.md frontmatter, and serves the merged list
//! through `GET /api/v1/repositories/{repository_id}/skills`.

pub mod error;
pub mod git_cache;
pub mod handlers;
pub mod models;
pub mod routes;
pub mod scanner;
pub mod schemas;
pub mod service;
