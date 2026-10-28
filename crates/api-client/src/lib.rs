//! Hand-written typed HTTP client for the Moatless API.

mod artifacts;
mod client;
mod collections;
mod error;
mod memory;

pub use artifacts::{ArtifactData, InlineArtifact, PageOptions};
pub use client::ApiClient;
pub use error::ApiError;
pub use memory::MemoryListQuery;

/// Wire contracts used by the Rust client.
pub mod types {
    pub use moatless_api_schemas::*;
    pub use moatless_workspace_types::schemas::{plugin, repository, workspace};
}
