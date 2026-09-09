//! Forgejo, addressed per instance.
//!
//! Forgejo is self-hostable, so nothing here is keyed on the provider alone: a
//! repository's git URL names the host, and the host selects the OAuth2
//! application, the `provider_tokens` row and the stored secret. Issuing a
//! token for a git operation is `crate::git_host_tokens`; this module registers
//! the applications and administers the credentials it reads.

pub mod access;
pub mod api;
pub(crate) mod dao;
pub mod instances;
pub mod oauth;

pub use access::{ForgejoAccess, ForgejoProviderTokenStatus};
pub use instances::{ForgejoInstances, ForgejoOAuthApp, RegisteredForgejoInstance, normalize_host};
pub use oauth::{ForgejoOAuth, ForgejoOAuthTokens};
