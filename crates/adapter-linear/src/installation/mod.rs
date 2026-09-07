//! Linear workspace OAuth installation.
mod client;
mod handlers;
mod service;
mod state;
mod routes;

pub use state::Installation;
pub use routes::router;
