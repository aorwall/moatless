//! Task domain types — typed enums and DB row structs.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::shared_models::AgentType;

// =============================================================================
// TaskStatus
// =============================================================================

/// Task lifecycle status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Open,
    Closed,
    Error,
}

impl TaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
            Self::Error => "error",
        }
    }
}

impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for TaskStatus {
    type Error = String;
    fn try_from(s: &str) -> Result<Self, String> {
        match s {
            "open" => Ok(Self::Open),
            "closed" => Ok(Self::Closed),
            "error" => Ok(Self::Error),
            other => Err(format!("unknown TaskStatus: {other:?}")),
        }
    }
}

impl sqlx::Type<sqlx::Postgres> for TaskStatus {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        <String as sqlx::Type<sqlx::Postgres>>::type_info()
    }
}

impl<'r> sqlx::Decode<'r, sqlx::Postgres> for TaskStatus {
    fn decode(
        value: sqlx::postgres::PgValueRef<'r>,
    ) -> Result<Self, sqlx::error::BoxDynError> {
        let s = <String as sqlx::Decode<sqlx::Postgres>>::decode(value)?;
        TaskStatus::try_from(s.as_str()).map_err(Into::into)
    }
}

impl sqlx::Encode<'_, sqlx::Postgres> for TaskStatus {
    fn encode_by_ref(
        &self,
        buf: &mut sqlx::postgres::PgArgumentBuffer,
    ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
        <&str as sqlx::Encode<sqlx::Postgres>>::encode_by_ref(&self.as_str(), buf)
    }
}

// =============================================================================
// Visibility
// =============================================================================

/// Task visibility level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    Public,
    Private,
}

impl Visibility {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Private => "private",
        }
    }
}

impl std::fmt::Display for Visibility {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for Visibility {
    type Error = String;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s {
            "public" => Ok(Self::Public),
            "private" => Ok(Self::Private),
            other => Err(format!("unknown Visibility: {other:?}")),
        }
    }
}

impl sqlx::Type<sqlx::Postgres> for Visibility {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        <String as sqlx::Type<sqlx::Postgres>>::type_info()
    }
}

impl<'r> sqlx::Decode<'r, sqlx::Postgres> for Visibility {
    fn decode(
        value: sqlx::postgres::PgValueRef<'r>,
    ) -> Result<Self, sqlx::error::BoxDynError> {
        let s = <String as sqlx::Decode<sqlx::Postgres>>::decode(value)?;
        Visibility::try_from(s.as_str()).map_err(Into::into)
    }
}

impl sqlx::Encode<'_, sqlx::Postgres> for Visibility {
    fn encode_by_ref(
        &self,
        buf: &mut sqlx::postgres::PgArgumentBuffer,
    ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
        <&str as sqlx::Encode<sqlx::Postgres>>::encode_by_ref(&self.as_str(), buf)
    }
}

// =============================================================================
// Task (DB)
// =============================================================================

/// Full task row from the `tasks` table.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct Task {
    pub task_id: String,
    pub short_id: Option<String>,
    pub created_at: String,
    pub last_message_at: String,
    pub agent_type: AgentType,
    pub repository_id: String,
    pub status: TaskStatus,
    pub total_cost_usd: f64,
    pub site_url: Option<String>,
    pub template_id: Option<String>,
    pub vibe: Option<bool>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub site_id: Option<String>,
    pub site_provider: Option<String>,
    pub branch: Option<String>,
    pub created_by: Option<String>,
    pub visibility: Visibility,
    pub created_at_db: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_status_serde_roundtrip() {
        assert_eq!(serde_json::to_string(&TaskStatus::Open).unwrap(), "\"open\"");
        assert_eq!(serde_json::to_string(&TaskStatus::Closed).unwrap(), "\"closed\"");
        assert_eq!(serde_json::to_string(&TaskStatus::Error).unwrap(), "\"error\"");

        let parsed: TaskStatus = serde_json::from_str("\"open\"").unwrap();
        assert_eq!(parsed, TaskStatus::Open);
    }

    #[test]
    fn visibility_serde_roundtrip() {
        assert_eq!(serde_json::to_string(&Visibility::Public).unwrap(), "\"public\"");
        assert_eq!(serde_json::to_string(&Visibility::Private).unwrap(), "\"private\"");

        let parsed: Visibility = serde_json::from_str("\"private\"").unwrap();
        assert_eq!(parsed, Visibility::Private);
    }

    #[test]
    fn task_status_try_from() {
        assert_eq!(TaskStatus::try_from("open"), Ok(TaskStatus::Open));
        assert!(TaskStatus::try_from("invalid").is_err());
    }

    #[test]
    fn visibility_try_from() {
        assert_eq!(Visibility::try_from("public"), Ok(Visibility::Public));
        assert!(Visibility::try_from("invalid").is_err());
    }
}
