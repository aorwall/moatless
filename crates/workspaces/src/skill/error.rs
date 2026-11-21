//! Skill error types.

#[derive(Debug, thiserror::Error)]
pub enum SkillError {
    #[error("Repository not found: {0}")]
    RepositoryNotFound(String),

    #[error("Workspace not found: {0}")]
    WorkspaceNotFound(String),

    #[error("Authentication required to fetch {git_url}")]
    AuthRequired { git_url: String },

    #[error("Unsupported git provider for {git_url} (only GitHub and Forgejo are supported)")]
    UnsupportedProvider { git_url: String },

    #[error("Skill fetch failed for {git_url}: {message}")]
    Fetch { git_url: String, message: String },

    #[error("Database error: {0}")]
    Db(#[from] sqlx::Error),

    /// Resolving the git-host token a skill or plugin fetch needs failed. A
    /// plain "not connected" is not an error and never reaches here.
    #[error("Repository access token could not be resolved")]
    RepositoryToken(#[source] crate::repository::error::RepositoryError),

    /// An invariant this module synthesizes, with no underlying failure to
    /// carry — kept distinct from the wrapping variants above.
    #[error("Internal error: {0}")]
    Internal(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_resolution_keeps_the_underlying_failure_as_source() {
        let err = SkillError::RepositoryToken(crate::repository::error::RepositoryError::Db(
            sqlx::Error::Protocol("SELECT access_token FROM provider_tokens".to_string()),
        ));
        let source = std::error::Error::source(&err).expect("repository error is the source");
        assert!(source.to_string().contains("SELECT access_token"));
    }

    #[test]
    fn synthesized_internal_messages_have_no_source() {
        let err = SkillError::Internal("skill cache not wired".to_string());
        assert!(std::error::Error::source(&err).is_none());
    }
}
