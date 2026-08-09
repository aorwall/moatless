//! Secret storage and the `secrets` CLI subcommand — the implementation lives in
//! the `moatless-identity` crate.
//!
//! Permanent facade (see `extractors.rs` for the rationale and the glob caveat).

pub use moatless_identity::secret::{audit, cli, dao, error, models, routes, schemas, service};

#[cfg(any(test, feature = "db-tests"))]
pub use moatless_identity::secret::test_support;
