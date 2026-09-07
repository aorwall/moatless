//! Deployment-wide Adapter app configuration.

pub mod dao;
mod app_dao;
mod models;
mod service;
#[cfg(test)]
mod tests;

pub use moatless_adapter_model::apps::{DEFAULT_APP_KEY, app_key_from_transport_config};
pub use models::{AdapterAppInput, AdapterAppSecretInput, StoredAdapterApp, LoadedAdapterApp};
pub use service::{app_id, upsert_adapter_app, carry_over_one, load_adapter_app, require_cipher};
pub(crate) use service::stored_app;
pub use app_dao::{list_adapter_apps, find_adapter_app, delete_adapter_app};

pub use models::SealedAdapterApp;
pub use service::save_adapter_app;
