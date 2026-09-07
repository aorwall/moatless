//! Report sandbox health and readiness through HTTP.

mod handlers;
mod routes;
pub(crate) use routes::router;
