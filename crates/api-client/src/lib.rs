//! Hand-written typed HTTP client for the Moatless API.

mod artifacts;
mod client;
mod error;

pub use artifacts::{ArtifactData, InlineArtifact, PageOptions};
pub use client::ApiClient;
pub use error::ApiError;
/// Wire contracts used by the Rust client.
pub mod types {
    pub use moatless_api_schemas::*;
    pub use moatless_workspace_types::schemas::{plugin, repository, workspace};
}
