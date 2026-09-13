//! Task-connections view — a cross-cutting HTTP module that joins a task, its
//! external bindings, the adapter registry (to name and status each
//! connection), and per-connection message counts. Server-band, owning its own
//! routes and `ApiDoc`; the same shape as `access_settings`, which likewise
//! composes several lower-band domains behind one router.

pub mod handlers;
pub mod routes;

mod projection;
mod query;
