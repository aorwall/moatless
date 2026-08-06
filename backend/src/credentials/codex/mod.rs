//! Codex credential subsystem — OAuth protocol, scope addressing, refresh
//! service, and sweep loop. `auth.json` parsing lives in
//! `auth::codex_auth_json` and the scope-aware save path in
//! `provider_settings::codex_save`.

pub mod oauth;
pub mod refresh;
pub mod refresher;
pub mod scope;

pub use self::scope::CodexScope;
