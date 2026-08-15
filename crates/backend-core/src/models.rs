//! Canonical cross-module domain vocabularies.

use serde::{Deserialize, Serialize};

macro_rules! string_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident => $value:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name {
            $($variant),+
        }

        impl $name {
            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $value),+
                }
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl TryFrom<&str> for $name {
            type Error = String;

            fn try_from(value: &str) -> Result<Self, String> {
                match value {
                    $($value => Ok(Self::$variant)),+,
                    other => Err(format!("unknown {}: {other:?}", stringify!($name))),
                }
            }
        }

        #[cfg(feature = "database")]
        impl sqlx::Type<sqlx::Postgres> for $name {
            fn type_info() -> sqlx::postgres::PgTypeInfo {
                <String as sqlx::Type<sqlx::Postgres>>::type_info()
            }
        }

        #[cfg(feature = "database")]
        impl<'r> sqlx::Decode<'r, sqlx::Postgres> for $name {
            fn decode(
                value: sqlx::postgres::PgValueRef<'r>,
            ) -> Result<Self, sqlx::error::BoxDynError> {
                let value = <String as sqlx::Decode<sqlx::Postgres>>::decode(value)?;
                Self::try_from(value.as_str()).map_err(Into::into)
            }
        }

        #[cfg(feature = "database")]
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

/// Agent type identifier (open-ended, e.g. `claude-code`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
        f.write_str(self.as_str())
    }
}

/// Agent mode identifier (open-ended, e.g. `plan` or `code`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AgentMode(pub String);

impl AgentMode {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for AgentMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

macro_rules! string_newtype_sqlx {
    ($name:ident) => {
        #[cfg(feature = "database")]
        impl sqlx::Type<sqlx::Postgres> for $name {
            fn type_info() -> sqlx::postgres::PgTypeInfo {
                <String as sqlx::Type<sqlx::Postgres>>::type_info()
            }
        }

        #[cfg(feature = "database")]
        impl<'r> sqlx::Decode<'r, sqlx::Postgres> for $name {
            fn decode(
                value: sqlx::postgres::PgValueRef<'r>,
            ) -> Result<Self, sqlx::error::BoxDynError> {
                Ok(Self(<String as sqlx::Decode<sqlx::Postgres>>::decode(
                    value,
                )?))
            }
        }

        #[cfg(feature = "database")]
        impl sqlx::Encode<'_, sqlx::Postgres> for $name {
            fn encode_by_ref(
                &self,
                buf: &mut sqlx::postgres::PgArgumentBuffer,
            ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
                <&str as sqlx::Encode<sqlx::Postgres>>::encode_by_ref(&self.0.as_str(), buf)
            }
        }
    };
}

string_newtype_sqlx!(AgentType);
string_newtype_sqlx!(AgentMode);

string_enum!(
    /// Task lifecycle status.
    TaskStatus {
        Open => "open",
        Closed => "closed",
        Error => "error",
    }
);

string_enum!(
    /// Persisted application intent for a task's Sandbox lifecycle.
    SandboxDesiredState {
        Running => "running",
        Stopped => "stopped",
        Removed => "removed",
    }
);

string_enum!(
    /// Reason persisted Sandbox intent moved away from running.
    SandboxStopReason {
        User => "user",
        IdleReaper => "idle_reaper",
        Cleanup => "cleanup",
        System => "system",
        Error => "error",
    }
);

string_enum!(
    /// Type of Task Message.
    MessageType {
        Assistant => "assistant",
        User => "user",
        System => "system",
        Result => "result",
        Error => "error",
    }
);

string_enum!(
    /// Role of the Message sender.
    MessageRole {
        User => "user",
        Assistant => "assistant",
        System => "system",
    }
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_values_round_trip() {
        for (status, value) in [
            (TaskStatus::Open, "open"),
            (TaskStatus::Closed, "closed"),
            (TaskStatus::Error, "error"),
        ] {
            assert_eq!(status.as_str(), value);
            assert_eq!(TaskStatus::try_from(value), Ok(status));
        }
    }
}
