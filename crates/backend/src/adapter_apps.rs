//! Adapter-app registry — the implementation lives in the `moatless-identity` crate.
//!
//! Permanent facade (see `extractors.rs` for the rationale and the glob caveat).

pub use moatless_identity::adapter_apps::{
    AdapterAppInput, AdapterAppSecretInput, DEFAULT_APP_KEY, StoredAdapterApp, app_id,
    app_key_from_transport_config, carry_over_env_and_load_config, dao, delete_adapter_app,
    list_adapter_apps, upsert_adapter_app,
};
