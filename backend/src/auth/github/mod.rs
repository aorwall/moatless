//! GitHub App authentication: the registered Apps, and the tokens they mint.
//!
//! [`dao`] is what an App *is* — the row an admin registered, with its
//! signing key sealed. [`service`] is what an App can *do* — mint a JWT from
//! that key and exchange it for short-lived installation tokens. [`registry`]
//! joins the two, handing out a live service for an App named by key and
//! noticing when the stored record behind it has changed.
//!
//! GitHub App *login* (the OAuth user-authorization flow) is a different thing
//! and lives with the other OAuth providers, in [`crate::auth::oauth`].

pub mod dao;
pub mod registry;
pub mod service;
