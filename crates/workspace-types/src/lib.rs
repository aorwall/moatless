//! Shared Workspace, repository, and plugin values.

#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

#[path = "../../types/src/string_enum.rs"]
mod string_enum;
pub(crate) use string_enum::string_enum;

pub mod plugin;
pub mod repository;
#[cfg(feature = "schemas")]
pub mod schemas;
pub mod workspace;
