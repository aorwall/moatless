//! The `adapter_connections` table, and nothing that decides over it.
//!
//! Private to the crate: a caller outside it takes one of the two handles in
//! [`crate::connections`], which carry the authority each operation runs under.

mod postgres;
mod row;

pub(crate) use postgres::PgAdapterConnectionDao;
