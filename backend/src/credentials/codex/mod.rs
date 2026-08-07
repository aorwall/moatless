//! Codex credential subsystem — OAuth protocol and the pure refresh path.
//! Row-addressing (`CodexScope`) lives in `auth::codex_scope`, `auth.json`
//! parsing in `auth::codex_auth_json`, and the scope-aware save path in
//! `provider_settings::codex_save`. The in-flight guard, sandbox broadcast and
//! background sweep moved up to the tasks-band `credential_sync` module (9c).

pub mod oauth;
pub mod refresh;
