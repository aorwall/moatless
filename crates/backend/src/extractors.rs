//! Request extractors — the types live in the `moatless-core` crate.
//!
//! This is a permanent facade: it lets backend call sites keep naming
//! `crate::extractors::{AuthUser, …}`, while the backend stays the composition
//! root that wires the concrete verifier into `AppState`
//! (`FromRef<AppState> for Arc<dyn extractors::UserTokenVerifier>` in `state.rs`).
//! Re-exports are explicit, not a glob: the cross-crate census resolves an edge
//! through an explicit re-export and drops a glob-through-facade.

pub use moatless_core::http_auth::{
    AuthError, AuthUser, MaybeAuthUser, OptionalScope, TaskAccess, TokenVerifyError,
    UserTokenVerifier, VerifiedUserToken,
};
