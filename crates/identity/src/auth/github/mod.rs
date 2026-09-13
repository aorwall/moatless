//! GitHub App authentication: the services that mint App tokens.
//!
//! GitHub bot App records live in `crate::adapter_apps::dao`. [`service`] is
//! what an App can *do* — mint a JWT and exchange it for installation tokens;
//! [`registry`] joins the two, keyed by App key.
//!
//! GitHub App *login* is a different thing, living with other OAuth providers
//! in [`crate::auth::oauth`].

pub mod registry;
pub mod service;
pub(crate) mod user_credential;
