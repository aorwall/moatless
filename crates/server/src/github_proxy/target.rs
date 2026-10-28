//! The repository a proxied GitHub request names, which its token is minted
//! for.

use moatless_workspace_types::repository::GitHostRepo;
use serde::Deserialize;

/// The repository a REST `path_and_query` addresses: `/repos/{owner}/{repo}…`.
pub fn from_path(path_and_query: &str) -> Option<GitHostRepo> {
    let path = path_and_query.split('?').next()?;
    let mut segments = path.strip_prefix("/repos/")?.split('/');
    repository(segments.next()?, segments.next()?)
}

#[derive(Deserialize)]
struct GraphqlRequest {
    #[serde(default)]
    variables: Option<GraphqlVariables>,
}

/// `gh` names a repository as `$owner` with `$repo`, or with `$name` in the
/// `repo` commands.
#[derive(Deserialize)]
struct GraphqlVariables {
    owner: Option<String>,
    repo: Option<String>,
    name: Option<String>,
}

/// The repository a GraphQL request's variables name.
///
/// A mutation addressing a node by id names none.
pub fn from_graphql(body: &[u8]) -> Option<GitHostRepo> {
    let variables = serde_json::from_slice::<GraphqlRequest>(body)
        .ok()?
        .variables?;
    repository(
        variables.owner.as_deref()?,
        variables.repo.or(variables.name).as_deref()?,
    )
}

fn repository(owner: &str, name: &str) -> Option<GitHostRepo> {
    let valid = |part: &str| {
        !part.is_empty()
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    (valid(owner) && valid(name)).then(|| GitHostRepo::github(owner, name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slug(repo: Option<GitHostRepo>) -> Option<String> {
        repo.map(|repo| repo.to_string())
    }

    #[test]
    fn a_rest_path_names_the_repository_after_repos() {
        assert_eq!(
            slug(from_path(
                "/repos/soaplabs/soap-platform-ci/pulls?state=open"
            )),
            Some("soaplabs/soap-platform-ci".to_string())
        );
        assert_eq!(
            slug(from_path("/repos/soaplabs/soap.frontend")),
            Some("soaplabs/soap.frontend".to_string())
        );
        assert_eq!(slug(from_path("/user")), None);
        assert_eq!(slug(from_path("/repos/soaplabs")), None);
        assert_eq!(slug(from_path("/repos/soaplabs/%2e%2e")), None);
    }

    #[test]
    fn a_graphql_request_names_the_repository_in_its_variables() {
        let body = |variables: serde_json::Value| {
            serde_json::to_vec(&serde_json::json!({ "query": "q", "variables": variables }))
                .unwrap()
        };
        assert_eq!(
            slug(from_graphql(&body(
                serde_json::json!({ "owner": "soaplabs", "repo": "soap-specs", "pr_number": 3 })
            ))),
            Some("soaplabs/soap-specs".to_string())
        );
        assert_eq!(
            slug(from_graphql(&body(
                serde_json::json!({ "owner": "soaplabs", "name": "soap-tm" })
            ))),
            Some("soaplabs/soap-tm".to_string())
        );
        assert_eq!(
            slug(from_graphql(&body(
                serde_json::json!({ "input": { "pullRequestId": "PR_kw" } })
            ))),
            None
        );
        assert_eq!(
            slug(from_graphql(b"{\"query\":\"{ viewer { login } }\"}")),
            None
        );
        assert_eq!(slug(from_graphql(b"not json")), None);
    }
}
