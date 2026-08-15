//! Repository request/response types.
//!
//! Re-exported from the `moatless-api-schemas` crate.

pub use moatless_api_schemas::repository::*;

use super::models::{Repository, RepositoryKind, RepositoryProvider};

pub(crate) fn scope_from_wire(
    scope: moatless_api_schemas::enums::Scope,
) -> moatless_backend_core::scope::Scope {
    match scope {
        moatless_api_schemas::enums::Scope::Global => moatless_backend_core::scope::Scope::Global,
        moatless_api_schemas::enums::Scope::User => moatless_backend_core::scope::Scope::User,
    }
}

pub(crate) fn scope_to_wire(
    scope: moatless_backend_core::scope::Scope,
) -> moatless_api_schemas::enums::Scope {
    match scope {
        moatless_backend_core::scope::Scope::Global => moatless_api_schemas::enums::Scope::Global,
        moatless_backend_core::scope::Scope::User => moatless_api_schemas::enums::Scope::User,
    }
}

pub(crate) fn repository_kind_from_wire(
    kind: moatless_api_schemas::enums::RepositoryKind,
) -> RepositoryKind {
    match kind {
        moatless_api_schemas::enums::RepositoryKind::Git => RepositoryKind::Git,
        moatless_api_schemas::enums::RepositoryKind::Remote => RepositoryKind::Remote,
        moatless_api_schemas::enums::RepositoryKind::Template => RepositoryKind::Template,
    }
}

fn repository_kind_to_wire(kind: RepositoryKind) -> moatless_api_schemas::enums::RepositoryKind {
    match kind {
        RepositoryKind::Git => moatless_api_schemas::enums::RepositoryKind::Git,
        RepositoryKind::Remote => moatless_api_schemas::enums::RepositoryKind::Remote,
        RepositoryKind::Template => moatless_api_schemas::enums::RepositoryKind::Template,
    }
}

pub(crate) fn repository_provider_from_wire(
    provider: moatless_api_schemas::enums::RepositoryProvider,
) -> RepositoryProvider {
    match provider {
        moatless_api_schemas::enums::RepositoryProvider::Github => RepositoryProvider::Github,
        moatless_api_schemas::enums::RepositoryProvider::Gitlab => RepositoryProvider::Gitlab,
        moatless_api_schemas::enums::RepositoryProvider::Gitness => RepositoryProvider::Gitness,
    }
}

fn repository_provider_to_wire(
    provider: RepositoryProvider,
) -> moatless_api_schemas::enums::RepositoryProvider {
    match provider {
        RepositoryProvider::Github => moatless_api_schemas::enums::RepositoryProvider::Github,
        RepositoryProvider::Gitlab => moatless_api_schemas::enums::RepositoryProvider::Gitlab,
        RepositoryProvider::Gitness => moatless_api_schemas::enums::RepositoryProvider::Gitness,
    }
}

impl From<Repository> for RepositoryResponse {
    fn from(r: Repository) -> Self {
        Self {
            id: r.id,
            name: r.name,
            kind: repository_kind_to_wire(r.kind),
            remote_url: r.remote_url,
            default_branch: r.default_branch,
            description: r.description,
            provider: r.provider.map(repository_provider_to_wire),
            scope: scope_to_wire(r.scope),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_repository_discriminant_maps_both_ways_with_identical_json() {
        for domain in [
            RepositoryKind::Git,
            RepositoryKind::Remote,
            RepositoryKind::Template,
        ] {
            let wire = repository_kind_to_wire(domain);
            assert_eq!(repository_kind_from_wire(wire), domain);
            assert_eq!(
                serde_json::to_value(domain).expect("serialize"),
                serde_json::to_value(wire).expect("serialize")
            );
        }
        for domain in [
            RepositoryProvider::Github,
            RepositoryProvider::Gitlab,
            RepositoryProvider::Gitness,
        ] {
            let wire = repository_provider_to_wire(domain);
            assert_eq!(repository_provider_from_wire(wire), domain);
            assert_eq!(
                serde_json::to_value(domain).expect("serialize"),
                serde_json::to_value(wire).expect("serialize")
            );
        }
        for domain in [
            moatless_backend_core::scope::Scope::Global,
            moatless_backend_core::scope::Scope::User,
        ] {
            let wire = scope_to_wire(domain);
            assert_eq!(scope_from_wire(wire), domain);
            assert_eq!(
                serde_json::to_value(domain).expect("serialize"),
                serde_json::to_value(wire).expect("serialize")
            );
        }
    }
}
