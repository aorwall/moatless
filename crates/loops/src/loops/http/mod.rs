//! Loop HTTP routes and response projections.

pub mod handlers;
pub(crate) mod projection;
mod routes;
pub use routes::{LoopsApiDoc, loops_router};
