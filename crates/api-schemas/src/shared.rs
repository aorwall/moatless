//! Shared newtype wrappers for cross-module identifiers.

use serde::{Deserialize, Serialize};

// =============================================================================
// AgentType
// =============================================================================

/// Agent type identifier (open-ended, e.g. "claude-code").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(transparent)]
pub struct AgentType(pub String);

impl AgentType {
    pub const CLAUDE_CODE: &str = "claude-code";
    pub const CLAUDE_CODE_TUI: &str = "claude-code-tui";
    pub const CLAUDE_CODE_CLI: &str = "claude-code-cli";
    pub const CLAUDE_CODE_FABLE: &str = "claude-code-fable";
    pub const CODEX: &str = "codex";
    pub const SUPPORTED: &'static [&'static str] = &[
        Self::CLAUDE_CODE,
        Self::CLAUDE_CODE_TUI,
        Self::CLAUDE_CODE_CLI,
        Self::CLAUDE_CODE_FABLE,
        Self::CODEX,
    ];

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for AgentType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(feature = "database")]
impl sqlx::Type<sqlx::Postgres> for AgentType {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        <String as sqlx::Type<sqlx::Postgres>>::type_info()
    }
}

#[cfg(feature = "database")]
impl<'r> sqlx::Decode<'r, sqlx::Postgres> for AgentType {
    fn decode(value: sqlx::postgres::PgValueRef<'r>) -> Result<Self, sqlx::error::BoxDynError> {
        let s = <String as sqlx::Decode<sqlx::Postgres>>::decode(value)?;
        Ok(Self(s))
    }
}

#[cfg(feature = "database")]
impl sqlx::Encode<'_, sqlx::Postgres> for AgentType {
    fn encode_by_ref(
        &self,
        buf: &mut sqlx::postgres::PgArgumentBuffer,
    ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
        <&str as sqlx::Encode<sqlx::Postgres>>::encode_by_ref(&self.0.as_str(), buf)
    }
}

// =============================================================================
// AgentMode
// =============================================================================

/// Agent mode identifier (open-ended, e.g. "plan", "code").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(transparent)]
pub struct AgentMode(pub String);

impl AgentMode {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for AgentMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(feature = "database")]
impl sqlx::Type<sqlx::Postgres> for AgentMode {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        <String as sqlx::Type<sqlx::Postgres>>::type_info()
    }
}

#[cfg(feature = "database")]
impl<'r> sqlx::Decode<'r, sqlx::Postgres> for AgentMode {
    fn decode(value: sqlx::postgres::PgValueRef<'r>) -> Result<Self, sqlx::error::BoxDynError> {
        let s = <String as sqlx::Decode<sqlx::Postgres>>::decode(value)?;
        Ok(Self(s))
    }
}

#[cfg(feature = "database")]
impl sqlx::Encode<'_, sqlx::Postgres> for AgentMode {
    fn encode_by_ref(
        &self,
        buf: &mut sqlx::postgres::PgArgumentBuffer,
    ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
        <&str as sqlx::Encode<sqlx::Postgres>>::encode_by_ref(&self.0.as_str(), buf)
    }
}
