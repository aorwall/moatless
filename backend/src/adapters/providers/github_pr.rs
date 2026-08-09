//! Permanent facade re-exporting the GitHub PR Adapter from `moatless-adapter-github-pr`.

pub use moatless_adapter_github_pr::{
    adapter, api_routes, delivery, handlers, router, shell_reconcile,
};
