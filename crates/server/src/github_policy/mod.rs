//! What a sandbox may write through the GitHub API and git proxies.
//!
//! Reads stay open to whatever the minted token reaches. A write must name a
//! repository the Workspace places or its grant's `githubPolicy.extraRepos`
//! lists, and fall in the allowed set; approving, merging, pushing tags and
//! pushing the default branch each need a grant capability. Under
//! `GITHUB_PROXY_POLICY_MODE=log` a refusal is logged and the request goes on.

mod graphql;
mod policy;
pub(crate) mod push;
mod refusal;
mod rest;
mod scope;
#[cfg(test)]
pub(crate) mod test_support;

pub use policy::GitHubPolicy;
pub(crate) use policy::Request;
pub(crate) use refusal::Refusal;
pub(crate) use scope::repo_key;
