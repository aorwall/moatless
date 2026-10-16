//! Database connection pools and health metrics.

use sqlx::postgres::PgPoolOptions;

pub type DbPool = sqlx::PgPool;

/// Check whether the pool can execute a database round trip.
pub async fn probe(pool: &DbPool) -> Result<i32, sqlx::Error> {
    sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(pool)
        .await
}

const POOL_CONNECTIONS: &str = "moatless_database_pool_connections";
const READINESS_FAILURES: &str = "moatless_database_readiness_failures_total";

/// How a pool reclaims idle connections.
///
/// The main pool trims idle connections after ten minutes; the dedicated
/// lock-only pool ([`PoolKind::Lock`]) never does, since a held session
/// advisory lock pins its connection for the whole operation and idle-reaping
/// it would drop the lock.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PoolKind {
    Main,
    Lock,
    /// [`create_probe_pool`]'s: its one connection is kept open, so a probe
    /// never pays for connecting.
    Probe,
}

/// Create a connection pool to the PostgreSQL database.
pub async fn create_pool(database_url: &str, max_connections: u32) -> Result<DbPool, sqlx::Error> {
    create_pool_kind(database_url, max_connections, PoolKind::Main).await
}

/// Create a pool with kind-specific defaults, so the main and lock pools share
/// one constructor and their common settings cannot drift.
pub async fn create_pool_kind(
    database_url: &str,
    max_connections: u32,
    kind: PoolKind,
) -> Result<DbPool, sqlx::Error> {
    pool_options(max_connections, kind)
        .connect(database_url)
        .await
}

fn pool_options(max_connections: u32, kind: PoolKind) -> PgPoolOptions {
    let opts = PgPoolOptions::new()
        .max_connections(max_connections)
        .acquire_timeout(std::time::Duration::from_secs(5));
    match kind {
        PoolKind::Main => opts.idle_timeout(std::time::Duration::from_mins(10)),
        PoolKind::Lock => opts,
        PoolKind::Probe => opts.min_connections(max_connections),
    }
}

/// Create the readiness probe's own one-connection pool to `main`'s database.
///
/// Separate deliberately: probing from a saturated `main` times out, and
/// Kubernetes then moves the load onto the other replica rather than letting
/// it drain. Lazy, so a database that is down fails the probe, not startup.
pub fn create_probe_pool(main: &DbPool) -> DbPool {
    pool_options(1, PoolKind::Probe).connect_lazy_with((*main.connect_options()).clone())
}

/// Register and refresh the main pool's operator-facing metrics.
///
/// SQLx exposes these values synchronously, so readiness can report pool
/// starvation without needing the connection whose absence it is diagnosing.
pub fn record_pool_metrics(pool: &DbPool) {
    let size = pool.size() as usize;
    let idle = pool.num_idle();
    let in_use = size.saturating_sub(idle);
    let max = pool.options().get_max_connections();

    metrics::describe_gauge!(
        POOL_CONNECTIONS,
        "SQLx main database pool connections by state"
    );
    metrics::describe_counter!(READINESS_FAILURES, "Database readiness failures by reason");
    metrics::gauge!(POOL_CONNECTIONS, "state" => "open").set(size as f64);
    metrics::gauge!(POOL_CONNECTIONS, "state" => "idle").set(idle as f64);
    metrics::gauge!(POOL_CONNECTIONS, "state" => "in_use").set(in_use as f64);
    metrics::gauge!(POOL_CONNECTIONS, "state" => "max").set(f64::from(max));
}

/// Count one failed database readiness check.
pub fn record_readiness_failure(reason: &'static str) {
    metrics::counter!(READINESS_FAILURES, "reason" => reason).increment(1);
}
