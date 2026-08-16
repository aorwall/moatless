//! Carrying a git-host token failure into this band's error.

use moatless_identity::git_host_tokens::GitHostTokenError;

use super::error::RepositoryError;

/// Carry a failed token resolution into this band's error without flattening
/// it: "you have not connected GitHub", "your credential cannot reach this
/// repository" and "your credential is broken" have three different remedies,
/// and the seam already tells them apart.
pub fn git_host_token_error(error: GitHostTokenError) -> RepositoryError {
    match error {
        GitHostTokenError::NoCredential { .. } => RepositoryError::BadRequest(error.to_string()),
        GitHostTokenError::Forbidden { .. } => RepositoryError::Forbidden(error.to_string()),
        GitHostTokenError::Broken { .. } => RepositoryError::ProviderError(error.to_string()),
        GitHostTokenError::Database(e) => RepositoryError::Db(e),
    }
}
