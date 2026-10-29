//! The backend's git smart-HTTP proxy.
//!
//! A Forgejo credential is never delivered to a pod. The pod clones from
//! `{BACKEND_API_URL}/git/{repository_id}` presenting its own task token, and
//! this module resolves that to the repository's real remote and the token the
//! task's run-as identity holds for it. `/git/github/{owner}/{name}` does the
//! same for any github.com repository, so a pod reaches GitHub holding no token.

mod body;
#[cfg(test)]
mod e2e_tests;
mod handlers;
mod routes;
mod service;

pub use routes::git_proxy_router;
pub use service::GitProxy;
