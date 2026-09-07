use crate::repository::error::RepositoryError;
use moatless_core::secret_keys::looks_like_secret_key;

pub(crate) fn validate_non_secret_env_vars(
    env_vars: Option<&std::collections::HashMap<String, String>>,
) -> Result<(), RepositoryError> {
    let Some(env_vars) = env_vars else {
        return Ok(());
    };
    for key in env_vars.keys() {
        if looks_like_secret_key(key) {
            return Err(RepositoryError::BadRequest(format!(
                "Repository env_vars are non-secret config only; move {key} to scoped secrets"
            )));
        }
    }
    Ok(())
}
