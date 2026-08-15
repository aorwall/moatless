//! Repository domain types — typed enums and DB row structs.

use chrono::{DateTime, Utc};

use moatless_backend_core::scope::Scope;

pub use crate::workspace::models::{ResourceConfig, ScriptConfig, ServerConfig};

macro_rules! repository_enum {
    ($name:ident { $($variant:ident => $value:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { $($variant),+ }

        impl $name {
            pub fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $value),+ }
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl TryFrom<&str> for $name {
            type Error = String;

            fn try_from(value: &str) -> Result<Self, Self::Error> {
                match value {
                    $($value => Ok(Self::$variant)),+,
                    other => Err(format!("unknown {}: {other:?}", stringify!($name))),
                }
            }
        }

        impl sqlx::Type<sqlx::Postgres> for $name {
            fn type_info() -> sqlx::postgres::PgTypeInfo {
                <String as sqlx::Type<sqlx::Postgres>>::type_info()
            }
        }

        impl<'r> sqlx::Decode<'r, sqlx::Postgres> for $name {
            fn decode(value: sqlx::postgres::PgValueRef<'r>) -> Result<Self, sqlx::error::BoxDynError> {
                let value = <String as sqlx::Decode<sqlx::Postgres>>::decode(value)?;
                Self::try_from(value.as_str()).map_err(Into::into)
            }
        }

        impl sqlx::Encode<'_, sqlx::Postgres> for $name {
            fn encode_by_ref(
                &self,
                buf: &mut sqlx::postgres::PgArgumentBuffer,
            ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
                <&str as sqlx::Encode<sqlx::Postgres>>::encode_by_ref(&self.as_str(), buf)
            }
        }
    };
}

repository_enum!(RepositoryKind {
    Git => "git",
    Remote => "remote",
    Template => "template",
});

repository_enum!(RepositoryProvider {
    Github => "github",
    Gitlab => "gitlab",
    Gitness => "gitness",
});

// =============================================================================
// Repository (DB)
// =============================================================================

/// Repository row from the `repositories` table.
///
/// A repository is only a connection to a git repo. Run-configuration lives on
/// the workspace. Physical run-config columns remain on this table for database
/// compatibility and are excluded from this model (`SELECT *` + `FromRow`
/// ignore them).
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct Repository {
    pub id: String,
    pub name: String,
    pub kind: RepositoryKind,
    pub remote_url: Option<String>,
    pub default_branch: Option<String>,
    pub description: Option<String>,
    pub provider: Option<RepositoryProvider>,
    pub scope: Scope,
    pub owner_user_id: Option<String>,
    pub created_by: Option<String>,
    /// Provider reachability of `remote_url`, stamped when the repo is
    /// auto-created by a git-synced workspace: `unchecked` | `ok` | `not_found`
    /// | `forbidden` | `error`. Defaults to `unchecked`.
    pub access_status: String,
    /// When `access_status` was last stamped.
    pub access_checked_at: Option<DateTime<Utc>>,
    /// Human-readable detail for a non-`ok` `access_status`.
    pub access_error: Option<String>,
    pub deleted: Option<bool>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}
