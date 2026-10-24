//! Deployment-wide Adapter app configuration.

pub mod administration;
pub(crate) mod app_dao;
pub(crate) mod dao;
mod models;
mod service;
#[cfg(test)]
mod tests;

pub use moatless_adapter_model::apps::{DEFAULT_APP_KEY, app_key_from_transport_config};
pub use models::{AdapterAppInput, AdapterAppSecretInput, LoadedAdapterApp, StoredAdapterApp};
pub(crate) use service::stored_app;
pub use service::{app_id, load_adapter_app, require_cipher, upsert_adapter_app};

pub use models::SealedAdapterApp;
pub(crate) use service::save_adapter_app;
