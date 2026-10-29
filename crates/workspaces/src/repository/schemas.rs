//! Repository request/response types.
//!
//! Re-exported from the `moatless-workspace-types::schemas` module.

pub use moatless_workspace_types::schemas::repository::*;

use super::models::Repository;

/// Both types are foreign to this crate, so this is a free function rather than
/// a `From` impl.
pub fn repository_response(r: Repository) -> RepositoryResponse {
    {
        RepositoryResponse {
            id: r.id,
            name: r.name,
            kind: r.kind,
            remote: r.remote.map(|remote| RepositoryRemoteResponse {
                url: remote.url,
                host_repo: remote.host_repo.map(|repo| GitHostRepoResponse {
                    provider: repo.provider(),
                    host: repo.host().to_string(),
                    owner: repo.owner().to_string(),
                    name: repo.name().to_string(),
                }),
            }),
            default_branch: r.default_branch,
            description: r.description,
            provider: r.provider,
            scope: r.scope,
            owner_user_id: r.owner_user_id,
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
    fn response_exposes_resolved_facts_without_changing_the_original_url() {
        let repository = crate::test_support::make_repository_row("repo", "user");
        let response = serde_json::to_value(repository_response(repository)).unwrap();
        assert_eq!(
            response["remote"]["url"],
            "https://github.com/test/repo.git"
        );
        assert!(response["remote"].get("usesSsh").is_none());
        assert_eq!(response["remote"]["hostRepo"]["provider"], "github");
        assert_eq!(response["remote"]["hostRepo"]["owner"], "test");
        assert_eq!(response["remote"]["hostRepo"]["name"], "repo");
        assert!(response.get("remoteUrl").is_none());
        assert!(response.get("remoteInfo").is_none());
    }

    #[test]
    fn remote_contract_accepts_explicit_absence_but_rejects_incomplete_responses() {
        let repository = crate::test_support::make_repository_row("repo", "user");
        let mut response = serde_json::to_value(repository_response(repository)).unwrap();
        response["remote"] = serde_json::json!({"hostRepo": null});
        assert!(serde_json::from_value::<RepositoryResponse>(response.clone()).is_err());
        response["remote"] = serde_json::Value::Null;
        assert!(
            serde_json::from_value::<RepositoryResponse>(response.clone())
                .unwrap()
                .remote
                .is_none()
        );
        response.as_object_mut().unwrap().remove("remote");
        assert!(serde_json::from_value::<RepositoryResponse>(response).is_err());
    }
}
