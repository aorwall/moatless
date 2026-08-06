//! Codex credential subsystem — OAuth protocol, refresh service, and sweep
//! loop. Row-addressing (`CodexScope`) lives in `auth::codex_scope`, `auth.json`
//! parsing in `auth::codex_auth_json`, and the scope-aware save path in
//! `provider_settings::codex_save`.

pub mod oauth;
pub mod refresh;
pub mod refresher;
