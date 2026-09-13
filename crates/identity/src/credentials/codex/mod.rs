//! Codex credential subsystem: the OAuth protocol, the stored `auth.json` row
//! and the pure refresh path.
//!
//! Row-addressing (`CodexScope`) lives in `auth::codex_scope` and `auth.json`
//! parsing in `auth::codex_auth_json`. The in-flight guard, the sandbox
//! broadcast and the background sweep belong to the tasks-band
//! `credential_sync` module.

mod blob;
pub mod oauth;
pub mod refresh;

pub use blob::CodexCredential;
