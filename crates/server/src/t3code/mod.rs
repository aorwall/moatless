pub mod backend;
pub mod projection;

// `test` stays first so the census keeps scoring this as test-context; the
// feature exposes it to this crate's own integration tests.
#[cfg(any(test, feature = "db-tests"))]
mod test_support {
    /// The T3 read model over a pool, with its activity seam wired to the same
    /// pool. Every db-test that reaches the read model wants that pairing, so
    /// it is built here — the composition root — rather than repeated at each
    /// call site.
    #[must_use]
    pub fn conversations_over(
        pool: &moatless_core::db::DbPool,
    ) -> moatless_t3code::dao::PgConversationDao {
        moatless_t3code::dao::PgConversationDao::new(
            pool.clone(),
            std::sync::Arc::new(moatless_tasks::ports::Activities::new(pool.clone())),
        )
    }
}

#[cfg(any(test, feature = "db-tests"))]
pub use test_support::conversations_over;
