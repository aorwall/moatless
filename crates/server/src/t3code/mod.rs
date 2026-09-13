pub mod backend;
pub mod projection;
pub mod startup_scripts;

// `test` stays first so the census keeps scoring this as test-context; the
// feature exposes it to this crate's own integration tests.
#[cfg(any(test, feature = "db-tests"))]
mod test_support {
    /// The T3 read model over a pool, with its activity seam wired to the same
    /// pool. Built here, the composition root, so every db-test that reaches it
    /// gets the pairing.
    #[must_use]
    pub fn t3_graph_over(pool: &moatless_core::db::DbPool) -> moatless_t3code::T3Graph {
        let tasks = moatless_tasks::compose::task_graph(pool.clone());
        moatless_t3code::t3_graph(pool.clone(), tasks.activities())
    }
}

#[cfg(any(test, feature = "db-tests"))]
pub use test_support::t3_graph_over;
