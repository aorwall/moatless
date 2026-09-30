//! Architecture rules for the Moatless workspace.
//!
//! A rule reads the workspace and returns [`Violation`]s. `arch-report` prints
//! them and exits 0; `tests/architecture.rs` fails on ones absent from
//! `arch-baseline.json`, or on any of them under `ARCH_STRICT=1`.

pub mod adapter_boundaries;
pub mod agents_md_budget;
pub mod agents_symlink;
pub mod annotations;
pub mod band_order;
pub mod base_surface;
pub mod baseline;
pub mod clippy_fence;
pub mod closed_dependency_list;
pub mod comment_block_size;
pub mod composition_root;
pub mod conversation_writer;
pub mod declaration_only_roots;
pub mod dependency_privacy;
pub mod duplicate_comment_prose;
pub mod duplicate_names;
pub mod env_read_owner;
pub mod file_size;
pub mod floor_type_admission;
pub mod manifest_inheritance;
pub mod module_names;
pub mod persistence_export;
pub mod persistence_sql_location;
pub mod port_admission;
pub mod process_exit;
mod public_api;
pub mod route_registration;
pub mod row_stays_with_query;
pub mod session_line_parsing;
pub mod single_implementation_trait;
mod source_index;
mod sql;
pub mod store_privacy;
pub mod table_ownership;
pub mod transport_direction;
pub mod workspace;

mod runner;
mod violation;
pub use runner::{RULES, Rule, run_all};
pub use violation::Violation;

#[cfg(test)]
#[allow(clippy::expect_used, reason = "fixture I/O must succeed")]
pub(crate) mod fixtures;

#[cfg(test)]
#[allow(clippy::expect_used, reason = "fixture rules must run")]
mod regressions;

#[cfg(test)]
#[allow(clippy::expect_used, reason = "source fixtures must parse")]
mod source_roles_tests;
