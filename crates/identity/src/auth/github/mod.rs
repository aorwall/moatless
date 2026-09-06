//! GitHub App authentication: the services that mint App tokens.
//!
//! GitHub bot App records live in [`crate::adapter_apps::dao`]. [`service`] is
//! what an App can *do* — mint a JWT from its signing key and exchange it for
//! short-lived installation tokens. [`registry`] joins the two, handing out a
//! live service for an App named by key and noticing when the stored record
//! behind it has changed.
//!
//! GitHub App *login* (the OAuth user-authorization flow) is a different thing
//! and lives with the other OAuth providers, in [`crate::auth::oauth`].

pub mod registry;
pub mod service;
pub mod user_credential;
