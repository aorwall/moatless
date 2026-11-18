//! The backend's Forgejo API proxy.
//!
//! A pod holds no Forgejo token. It calls `/forgejo/{host}/api/v1/…` with its
//! task token, and the proxy presents the run-as identity's credential for
//! `host` instead, resolved per request so a refreshed OAuth token is the one
//! sent. `moat api` reaches it as it reaches any backend route.

mod handlers;
mod routes;
mod service;

pub use routes::forgejo_proxy_router;
pub use service::{ForgejoProxy, ForgejoProxyError};
