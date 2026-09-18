//! Linear workspace OAuth installation.
mod client;
mod handlers;
mod routes;
mod service;
mod state;

pub use routes::router;
pub use state::Installation;
