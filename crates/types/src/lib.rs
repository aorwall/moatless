//! Shared type definitions for Moatless.
//!
//! Every type here is pure data: no I/O, no behaviour beyond what the data
//! itself can answer.
//!
//! Admission is enforced by `tooling/arch-rules`; see `crates/types/AGENTS.md`.

// Test code asserts a parse succeeded; production code keeps the workspace deny.
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

/// A closed set of values persisted as text and carried on the wire as text.
///
/// Generates the wire mapping (`as_str`, `Display`, `TryFrom<&str>`,
/// `TryFrom<String>`) and nothing else. Reading one out of a database column
/// belongs to the crate that owns the query, not here.
macro_rules! string_enum {
    (
        $(#[$meta:meta])*
        $name:ident { $($(#[$vmeta:meta])* $variant:ident => $value:literal),+ $(,)? }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
        #[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
        #[serde(rename_all = "snake_case")]
        pub enum $name {
            $($(#[$vmeta])* $variant),+
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

        impl TryFrom<String> for $name {
            type Error = String;

            fn try_from(value: String) -> Result<Self, String> {
                Self::try_from(value.as_str())
            }
        }
    };
}

pub(crate) use string_enum;

pub mod access_control;
pub mod adapter;
pub mod agent;
pub mod auth;
pub mod event;
pub mod feedback;
pub mod loops;
pub mod message;
pub mod repository;
pub mod sandbox;
pub mod secret;
pub mod store;
pub mod task;
pub mod user;
pub mod workspace;

pub mod activity;
pub mod codex;
pub mod plugin;
pub mod running;
pub mod ui_rpc;
