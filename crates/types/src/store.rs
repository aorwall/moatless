//! The error every published store seam returns.
//!
//! `store` names the seam the error belongs to, not the kind of thing it is.
//! It sits on the floor so a crate below `backend-core` — the Adapter kernel,
//! for one — can return it without reaching up a band.

/// A store-layer failure. Carries a human context and, when the backend
/// provides one, the underlying source (e.g. a `sqlx::Error`), so the
/// operation-specific error a caller raises does not flatten it to an opaque
/// string. Boxing the source is what keeps a store driver out of a caller's
/// dependency list.
#[derive(Debug, thiserror::Error)]
#[error("{context}")]
pub struct StoreError {
    pub context: String,
    #[source]
    pub source: Option<Box<dyn std::error::Error + Send + Sync + 'static>>,
}

impl StoreError {
    pub fn new(context: impl Into<String>) -> Self {
        Self {
            context: context.into(),
            source: None,
        }
    }

    pub fn with_source(
        context: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            context: context.into(),
            source: Some(Box::new(source)),
        }
    }
}
