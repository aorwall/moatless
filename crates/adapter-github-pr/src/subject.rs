//! Canonical GitHub pull-request subject coordinates.

use moatless_loops::adapters::error::{AdapterError, SharedAdapterError};
use moatless_loops::adapters::models::{BindingContext, TaskBinding};
use moatless_loops::adapters::{AdapterDestinationBuilder, BindingDestination};

use super::adapter::GithubPrAdapter;

const SUBJECT_KIND: &str = "github_pr";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GithubPrSubject {
    owner: String,
    repo: String,
    pr_number: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GithubRepository {
    owner: String,
    repo: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MissingSubjectField {
    Owner,
    Repo,
    PrNumber,
}

impl MissingSubjectField {
    const fn name(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Repo => "repo",
            Self::PrNumber => "pr_number",
        }
    }
}

impl GithubPrSubject {
    pub(super) fn new(owner: &str, repo: &str, pr_number: u64) -> Self {
        Self {
            owner: owner.to_string(),
            repo: repo.to_string(),
            pr_number,
        }
    }

    pub(super) fn from_thread_properties(value: &serde_json::Value) -> Result<Self, AdapterError> {
        Self::parse(value).map_err(|field| {
            SharedAdapterError::BadRequest(format!(
                "Missing {} in GitHub PR destination",
                field.name()
            ))
            .into()
        })
    }

    pub(super) fn from_binding(binding: &TaskBinding) -> Result<Self, AdapterError> {
        Self::parse(&binding.destination_config).map_err(|field| {
            SharedAdapterError::Internal(format!(
                "Missing {} in binding destination_config",
                field.name()
            ))
            .into()
        })
    }

    pub(super) fn from_destination_config(value: &serde_json::Value) -> Option<Self> {
        Self::parse(value).ok()
    }

    pub(super) fn from_source_and_number(
        source_id: &str,
        pr_number: u64,
    ) -> Result<Self, AdapterError> {
        let (owner, repo) = source_id.split_once('/').ok_or_else(|| {
            SharedAdapterError::BadRequest(format!("Invalid GitHub source_matcher: {source_id}"))
        })?;
        Ok(Self::new(owner, repo, pr_number))
    }

    fn parse(value: &serde_json::Value) -> Result<Self, MissingSubjectField> {
        let owner = value
            .get("owner")
            .and_then(serde_json::Value::as_str)
            .ok_or(MissingSubjectField::Owner)?;
        let repo = value
            .get("repo")
            .and_then(serde_json::Value::as_str)
            .ok_or(MissingSubjectField::Repo)?;
        let pr_number = value
            .get("pr_number")
            .and_then(serde_json::Value::as_u64)
            .ok_or(MissingSubjectField::PrNumber)?;
        Ok(Self::new(owner, repo, pr_number))
    }

    pub(super) fn owner(&self) -> &str {
        &self.owner
    }

    pub(super) fn repo(&self) -> &str {
        &self.repo
    }

    pub(super) const fn pr_number(&self) -> u64 {
        self.pr_number
    }

    pub(super) fn key(&self) -> String {
        format!("{}/{}#{}", self.owner, self.repo, self.pr_number)
    }

    pub(super) fn status_key(&self) -> String {
        format!("gh:{}", self.key())
    }

    pub(super) fn pull_request_url(&self) -> String {
        format!(
            "https://github.com/{}/{}/pull/{}",
            self.owner, self.repo, self.pr_number
        )
    }

    pub(super) fn destination_config(&self, filter_user_id: Option<&str>) -> serde_json::Value {
        let mut config = serde_json::json!({
            "owner": self.owner,
            "repo": self.repo,
            "pr_number": self.pr_number,
        });
        if let Some(filter_user_id) = filter_user_id {
            config["filter_user_id"] = serde_json::json!(filter_user_id);
        }
        config
    }

    pub(super) fn binding_destination(&self, filter_user_id: Option<&str>) -> BindingDestination {
        BindingDestination::new(
            SUBJECT_KIND,
            self.key(),
            self.destination_config(filter_user_id),
        )
    }
}

impl GithubRepository {
    pub(super) fn from_binding_context(binding: &BindingContext) -> Result<Self, AdapterError> {
        let owner = binding
            .destination_config
            .get("owner")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                SharedAdapterError::BadRequest("Missing owner in binding destination_config".into())
            })?;
        let repo = binding
            .destination_config
            .get("repo")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                SharedAdapterError::BadRequest("Missing repo in binding destination_config".into())
            })?;
        Ok(Self {
            owner: owner.to_string(),
            repo: repo.to_string(),
        })
    }

    pub(super) fn owner(&self) -> &str {
        &self.owner
    }

    pub(super) fn repo(&self) -> &str {
        &self.repo
    }
}

impl AdapterDestinationBuilder for GithubPrAdapter {
    fn connection_scope_value_from_existing_thread(
        &self,
        source_id: &str,
        _thread_properties: &serde_json::Value,
    ) -> Result<String, AdapterError> {
        source_id
            .split('/')
            .next()
            .filter(|owner| !owner.is_empty())
            .map(str::to_string)
            .ok_or_else(|| {
                SharedAdapterError::BadRequest(format!(
                    "Invalid GitHub source_id (expected owner/repo): {source_id}"
                ))
                .into()
            })
    }

    fn allow_default_connection_fallback(&self) -> bool {
        false
    }

    fn allow_wildcard_trigger_fallback(&self) -> bool {
        false
    }

    fn binding_destination_from_existing_thread(
        &self,
        _source_id: &str,
        _external_thread_id: &str,
        thread_properties: &serde_json::Value,
        filter_user_id: Option<&str>,
    ) -> Result<BindingDestination, AdapterError> {
        let subject = GithubPrSubject::parse(thread_properties).map_err(|field| {
            SharedAdapterError::BadRequest(format!(
                "thread_properties missing required {}",
                field.name()
            ))
        })?;
        Ok(subject.binding_destination(filter_user_id))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use moatless_core::db::DbPool;
    use moatless_identity::auth::provider_token_dao::ProviderTokenDao;

    use super::*;
    use crate::adapter::GithubPrAdapter;
    use moatless_loops::adapters::AdapterDestinationBuilder;

    #[test]
    fn parses_thread_properties_and_builds_canonical_values() {
        let subject = GithubPrSubject::from_thread_properties(&serde_json::json!({
            "owner": "acme",
            "repo": "widgets",
            "pr_number": 42,
        }))
        .unwrap();

        assert_eq!(subject.owner(), "acme");
        assert_eq!(subject.repo(), "widgets");
        assert_eq!(subject.pr_number(), 42);
        assert_eq!(subject.key(), "acme/widgets#42");
        assert_eq!(subject.status_key(), "gh:acme/widgets#42");
        assert_eq!(
            subject.pull_request_url(),
            "https://github.com/acme/widgets/pull/42"
        );
        assert_eq!(
            subject.destination_config(Some("user-1")),
            serde_json::json!({
                "owner": "acme",
                "repo": "widgets",
                "pr_number": 42,
                "filter_user_id": "user-1",
            })
        );
    }

    #[test]
    fn reports_each_malformed_coordinate_fallibly() {
        for (value, expected) in [
            (
                serde_json::json!({"repo": "widgets", "pr_number": 42}),
                "Invalid request: Missing owner in GitHub PR destination",
            ),
            (
                serde_json::json!({"owner": "acme", "pr_number": 42}),
                "Invalid request: Missing repo in GitHub PR destination",
            ),
            (
                serde_json::json!({"owner": "acme", "repo": "widgets"}),
                "Invalid request: Missing pr_number in GitHub PR destination",
            ),
        ] {
            let error = GithubPrSubject::from_thread_properties(&value).unwrap_err();
            assert_eq!(error.to_string(), expected);
        }
    }

    #[tokio::test]
    async fn binding_destination_from_existing_thread_builds_pr_subject() {
        let provider = provider();
        let destination = provider
            .binding_destination_from_existing_thread(
                "foo/bar",
                "",
                &serde_json::json!({
                    "owner": "foo",
                    "repo": "bar",
                    "pr_number": 42,
                }),
                Some("user-1"),
            )
            .unwrap();

        assert_eq!(destination.subject_kind, "github_pr");
        assert_eq!(destination.subject_key, "foo/bar#42");
        assert_eq!(destination.destination_config["owner"], "foo");
        assert_eq!(destination.destination_config["repo"], "bar");
        assert_eq!(destination.destination_config["pr_number"], 42);
        assert_eq!(destination.destination_config["filter_user_id"], "user-1");
        assert_eq!(destination.last_external_event_id, None);
    }

    fn provider() -> GithubPrAdapter {
        GithubPrAdapter::new(
            None,
            "https://app.example.com".to_string(),
            moatless_identity::git_host_tokens::seam_over(fake_provider_tokens()),
        )
    }

    fn fake_db_pool() -> DbPool {
        sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect_lazy("postgres://fake:fake@localhost:1/fake")
            .unwrap()
    }

    fn fake_provider_tokens() -> Arc<dyn ProviderTokenDao> {
        Arc::new(moatless_identity::auth::provider_token_dao::PgProviderTokenDao(fake_db_pool()))
    }
}
