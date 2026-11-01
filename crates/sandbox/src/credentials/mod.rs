//! Credential acquisition, application and rotation.

pub mod broker;
mod dispatcher;
mod ports;
pub mod recovery;
pub mod sinks;
pub use dispatcher::Dispatcher;
pub use ports::CredentialSink;
pub(crate) mod fetch;
pub(crate) mod gh_config;
pub(crate) mod github_proxy;
pub(crate) mod refresher;
pub(crate) mod retry;
