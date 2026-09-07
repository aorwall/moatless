//! Adapter connection storage contracts and PostgreSQL implementation.

mod connection;
#[cfg(feature = "postgres")]
mod postgres;

pub use connection::AdapterConnectionDao;
#[cfg(feature = "postgres")]
pub use postgres::PgAdapterConnectionDao;
