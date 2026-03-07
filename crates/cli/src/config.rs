//! Configuration from environment variables.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Missing required environment variable: {0}")]
    Missing(String),
}

#[derive(Debug, Clone)]
pub struct Config {
    pub backend_api_url: String,
    pub user_auth_token: String,
    pub task_id: Option<String>,
    pub session_file: String,
    pub cookie_name: String,
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        let backend_api_url = std::env::var("BACKEND_API_URL")
            .map_err(|_| ConfigError::Missing("BACKEND_API_URL".to_string()))?;
        let user_auth_token = std::env::var("USER_AUTH_TOKEN")
            .map_err(|_| ConfigError::Missing("USER_AUTH_TOKEN".to_string()))?;
        let task_id = std::env::var("TASK_ID").ok();

        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        let session_file = format!("{home}/.moat_session");

        let cookie_name = std::env::var("MOAT_COOKIE_NAME")
            .unwrap_or_else(|_| "moatless_session".to_string());

        Ok(Self {
            backend_api_url,
            user_auth_token,
            task_id,
            session_file,
            cookie_name,
        })
    }
}
