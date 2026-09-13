//! Hand-written typed HTTP client for the Moatless API.

mod client;
mod error;

pub use client::ApiClient;
pub use error::ApiError;
/// Wire contracts used by the Rust client.
pub mod types {
    pub use moatless_api_schemas::*;
    pub use moatless_workspace_types::schemas::{plugin, repository, workspace};
}
