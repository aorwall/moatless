//! Permanent facade re-exporting the GitHub PR Adapter from
//! `moatless-adapter-github-pr`.

pub use moatless_adapter_github_pr::{
    adapter, api_routes, check_run_trigger, delivery, handlers, router,
};
