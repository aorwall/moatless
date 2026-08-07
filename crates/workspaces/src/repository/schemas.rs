//! Repository request/response types.
//!
//! Re-exported from the `moatless-api-schemas` crate.

pub use moatless_api_schemas::repository::*;

use super::models::Repository;

impl From<Repository> for RepositoryResponse {
    fn from(r: Repository) -> Self {
        Self {
            id: r.id,
            name: r.name,
            kind: r.kind,
            remote_url: r.remote_url,
            default_branch: r.default_branch,
            description: r.description,
            provider: r.provider,
            scope: r.scope,
            owner_user_id: r.owner_user_id,
            access_status: r.access_status,
            access_checked_at: r.access_checked_at.map(|t| t.to_rfc3339()),
            access_error: r.access_error,
            created_by: r.created_by,
            created_at: r.created_at.map(|t| t.to_rfc3339()),
            updated_at: r.updated_at.map(|t| t.to_rfc3339()),
        }
    }
}
