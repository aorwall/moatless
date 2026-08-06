//! Reserved secret-store key names that lower bands must name.
//!
//! `GITHUB_TOKEN` lives here because identity- and workspaces-band credential
//! code names it; the sibling provider keys stay in the sandbox module their
//! only consumers sit beside.

/// The reserved key name for a task's GitHub token secret.
pub const GITHUB_TOKEN: &str = "GITHUB_TOKEN";
