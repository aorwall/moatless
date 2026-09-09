//! Owner construction for the `adapter_connections` store.

use std::sync::Arc;

use sqlx::PgPool;

use crate::connections::ConnectionGraph;
use crate::connections::dao::PgAdapterConnectionDao;

/// Build both connection handles over the shared pool.
pub fn adapter_connections(pool: PgPool) -> ConnectionGraph {
    ConnectionGraph::over(Arc::new(PgAdapterConnectionDao::new(pool)))
}
