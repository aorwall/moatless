//! Repository domain types — typed enums and DB row structs.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::task::models::Visibility;

// =============================================================================
// RepositoryKind
// =============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RepositoryKind {
    Git,
    Remote,
    Template,
}

impl RepositoryKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Git => "git",
            Self::Remote => "remote",
            Self::Template => "template",
        }
    }
}

impl std::fmt::Display for RepositoryKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for RepositoryKind {
    type Error = String;
    fn try_from(s: &str) -> Result<Self, String> {
        match s {
            "git" => Ok(Self::Git),
            "remote" => Ok(Self::Remote),
            "template" => Ok(Self::Template),
            other => Err(format!("unknown RepositoryKind: {other:?}")),
        }
    }
}

impl sqlx::Type<sqlx::Postgres> for RepositoryKind {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        <String as sqlx::Type<sqlx::Postgres>>::type_info()
    }
}

impl<'r> sqlx::Decode<'r, sqlx::Postgres> for RepositoryKind {
    fn decode(
        value: sqlx::postgres::PgValueRef<'r>,
    ) -> Result<Self, sqlx::error::BoxDynError> {
        let s = <String as sqlx::Decode<sqlx::Postgres>>::decode(value)?;
        RepositoryKind::try_from(s.as_str()).map_err(Into::into)
    }
}

impl sqlx::Encode<'_, sqlx::Postgres> for RepositoryKind {
    fn encode_by_ref(
        &self,
        buf: &mut sqlx::postgres::PgArgumentBuffer,
    ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
        <&str as sqlx::Encode<sqlx::Postgres>>::encode_by_ref(&self.as_str(), buf)
    }
}

// =============================================================================
// RepositoryProvider
// =============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RepositoryProvider {
    Github,
    Gitlab,
}

impl RepositoryProvider {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Github => "github",
            Self::Gitlab => "gitlab",
        }
    }
}

impl std::fmt::Display for RepositoryProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for RepositoryProvider {
    type Error = String;
    fn try_from(s: &str) -> Result<Self, String> {
        match s {
            "github" => Ok(Self::Github),
            "gitlab" => Ok(Self::Gitlab),
            other => Err(format!("unknown RepositoryProvider: {other:?}")),
        }
    }
}

impl sqlx::Type<sqlx::Postgres> for RepositoryProvider {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        <String as sqlx::Type<sqlx::Postgres>>::type_info()
    }
}

impl<'r> sqlx::Decode<'r, sqlx::Postgres> for RepositoryProvider {
    fn decode(
        value: sqlx::postgres::PgValueRef<'r>,
    ) -> Result<Self, sqlx::error::BoxDynError> {
        let s = <String as sqlx::Decode<sqlx::Postgres>>::decode(value)?;
        RepositoryProvider::try_from(s.as_str()).map_err(Into::into)
    }
}

impl sqlx::Encode<'_, sqlx::Postgres> for RepositoryProvider {
    fn encode_by_ref(
        &self,
        buf: &mut sqlx::postgres::PgArgumentBuffer,
    ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
        <&str as sqlx::Encode<sqlx::Postgres>>::encode_by_ref(&self.as_str(), buf)
    }
}

// =============================================================================
// Repository (DB)
// =============================================================================

/// Repository row from the `repositories` table.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct Repository {
    pub id: String,
    pub name: String,
    pub kind: RepositoryKind,
    pub remote_url: Option<String>,
    pub default_branch: Option<String>,
    pub docker_image: Option<String>,
    pub system_prompt: Option<String>,
    pub setup_commands: Option<serde_json::Value>,
    pub env_vars: Option<serde_json::Value>,
    pub servers: Option<serde_json::Value>,
    pub plugins: Option<serde_json::Value>,
    // Preview config (flattened columns)
    pub server_start_commands: Option<serde_json::Value>,
    pub startup_timeout_seconds: Option<i32>,
    pub build_command: Option<String>,
    pub build_output_directory: Option<String>,
    pub enable_inspector: Option<bool>,
    pub auto_reload: Option<bool>,
    // CRUD fields
    pub description: Option<String>,
    pub provider: Option<RepositoryProvider>,
    pub visibility: Option<Visibility>,
    pub created_by: Option<String>,
    pub deleted: Option<bool>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}
