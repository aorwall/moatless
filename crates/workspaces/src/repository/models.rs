//! Persisted shapes for the repository band.
//!
//! The domain types live in `moatless_workspace_types::repository`. What is here is the
//! shape the `repositories` table has — a nullable text `provider` column and a
//! nullable `deleted` flag — plus the decode from one to the other.

use chrono::{DateTime, Utc};

use moatless_types::access_control::Scope;
// The repository band's vocabulary surface: call sites keep naming these
// through `repository::models`, which is where a reader of this crate looks.
pub use moatless_workspace_types::repository::{Repository, RepositoryKind, RepositoryProvider};

pub use moatless_workspace_types::workspace::ScriptConfig;

/// Patch struct for partial updates.
#[derive(Default)]
pub struct RepositoryPatch {
    pub name: Option<String>,
    pub kind: Option<RepositoryKind>,
    pub scope: Option<Scope>,
    pub owner_user_id: Option<Option<String>>,
    pub remote_url: Option<Option<String>>,
    pub default_branch: Option<Option<String>>,
    pub description: Option<Option<String>>,
    pub provider: Option<Option<RepositoryProvider>>,
}

// =============================================================================
// RepositoryRow
// =============================================================================

/// Row from the `repositories` table.
///
/// Physical run-config columns remain on this table for database compatibility
/// and are excluded from this model (`SELECT *` + `FromRow` ignore them).
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct RepositoryRow {
    pub id: String,
    pub name: String,
    #[sqlx(try_from = "String")]
    pub kind: RepositoryKind,
    pub remote_url: Option<String>,
    pub default_branch: Option<String>,
    pub description: Option<String>,
    /// Nullable enum columns decode as text.
    pub provider: Option<String>,
    #[sqlx(try_from = "String")]
    pub scope: Scope,
    pub owner_user_id: Option<String>,
    pub created_by: Option<String>,
    pub deleted: Option<bool>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

impl From<RepositoryRow> for Repository {
    fn from(row: RepositoryRow) -> Self {
        let provider = decode_provider(&row.id, row.provider.as_deref());
        let remote = row
            .remote_url
            .map(|url| super::remote::resolve_remote(provider, url));
        if provider.is_some()
            && remote
                .as_ref()
                .is_some_and(|remote| remote.host_repo.is_none())
        {
            tracing::warn!(
                repository_id = %row.id,
                ?provider,
                "Declared provider has no supported repository host identity; git-host operations are unavailable"
            );
        }
        Self {
            provider,
            id: row.id,
            name: row.name,
            kind: row.kind,
            remote,
            default_branch: row.default_branch,
            description: row.description,
            scope: row.scope,
            owner_user_id: row.owner_user_id,
            created_by: row.created_by,
            deleted: row.deleted.unwrap_or(false),
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// Read the `provider` column, logging a value this build does not know.
///
/// An unreadable provider reads as unset rather than failing the row, so one
/// stale value cannot make a repository disappear from every listing.
fn decode_provider(repository_id: &str, raw: Option<&str>) -> Option<RepositoryProvider> {
    match RepositoryProvider::try_from(raw?) {
        Ok(provider) => Some(provider),
        Err(error) => {
            tracing::warn!(%repository_id, %error, "unreadable provider column");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_provider_keeps_its_remote_without_a_host_identity() {
        let row = RepositoryRow {
            id: "repo".to_string(),
            name: "repo".to_string(),
            kind: RepositoryKind::Git,
            remote_url: Some("https://github.com/acme/repo.git".to_string()),
            default_branch: None,
            description: None,
            provider: Some("unrecognized".to_string()),
            scope: Scope::Global,
            owner_user_id: None,
            created_by: None,
            deleted: None,
            created_at: None,
            updated_at: None,
        };
        let repository = Repository::from(row);
        assert_eq!(repository.provider, None);
        let remote = repository.remote.unwrap();
        assert_eq!(remote.url, "https://github.com/acme/repo.git");
        assert!(remote.host_repo.is_none());
    }
}
