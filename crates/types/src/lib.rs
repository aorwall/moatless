//! Shared type definitions for Moatless.
//!
//! Every type here is pure data: no I/O, no behaviour beyond what the data
//! itself can answer. The crate depends on `serde`, `serde_json`, `chrono`,
//! `thiserror`, `tracing`, and optionally `utoipa`, and on nothing else — no
//! `sqlx`, no `axum`, no `reqwest`, no `redis`. Crates above it therefore never
//! rebuild because a transport dependency moved.
//!
//! ## What belongs here
//!
//! A type moves in when all three hold:
//!
//! - it is pure data;
//! - three or more crates other than its owner name it;
//! - neither its body nor its impls name a foreign crate.
//!
//! A type named by only one or two neighbours is that subsystem's vocabulary
//! and stays with the subsystem. A type whose impls reach a foreign crate —
//! an `sqlx::FromRow` row, an error with a `#[from] sqlx::Error` variant —
//! stays with the code that does the I/O.
//!
//! ## Domain models and rows
//!
//! An entity that clears the bar lives here as its domain model: every field
//! is the value the codebase reads, not the shape a column happens to have. A
//! timestamp is a `DateTime<Utc>`, a closed set is its enum, a JSON blob is
//! the type it decodes to, and a persistence artifact — a raw text twin, a
//! nullable flag standing in for a `bool` — is not a field at all.
//!
//! The row that backs it stays in the crate that owns the query, as `XRow`
//! with the `sqlx::FromRow` derive and a `From`/`TryFrom<XRow> for X` beside
//! it. That conversion is the one place a column is decoded, so a call site
//! never re-parses and a lenient read never turns into a silent default by
//! accident. Where a decode is deliberately lenient, its log is what makes it
//! recoverable; say so at the conversion.
//!
//! ## Where it goes
//!
//! The module is the domain, and the domain is named from `CONTEXT.md`. A
//! reader looking for a type opens the module for the thing the type is about,
//! not for the kind of thing it is: there is no `config`, no `error`, no
//! `kind` module, because none of those is a domain.
//!
//! A type sits with the domain that owns it even when a neighbouring module
//! reads more obvious at a glance. Say why in the module doc when it does.

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
pub mod task;
pub mod user;
pub mod workspace;
