//! The backend's GitHub API proxy.
//!
//! A pod's `gh` holds no GitHub token. It reaches the API through the
//! sandbox's `github.sock`, which forwards to `{BACKEND_API_URL}/github/…`
//! presenting the pod's own task token; this module resolves the token the
//! sandbox's run-as identity holds for the repository each request names, so
//! an installation token that expired since the last call is never the one
//! presented.

mod authorship;
mod handlers;
mod made;
mod routes;
mod service;
mod target;

pub use routes::github_proxy_router;
pub use service::{GITHUB_UPLOADS_BASE, GitHubProxy};
