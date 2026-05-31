//! Codex credential subsystem — OAuth protocol, scope addressing, refresh
//! service, sweep loop, and `auth.json` validation. Owns every codex-specific
//! concern that used to be scattered across `provider_settings/` and
//! `credentials/`.

pub mod auth_json;
pub mod oauth;
pub mod refresh;
pub mod refresher;
pub mod scope;

pub use self::auth_json::{build_codex_metadata, parse_codex_auth_json, save_codex_auth_for_scope};
pub use self::scope::{CodexScope, SYSTEM_TEAM_ID};
