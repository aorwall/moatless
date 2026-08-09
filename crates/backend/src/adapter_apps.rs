//! Adapter-app registry — the implementation lives in the `moatless-identity` crate.
//!
//! Permanent facade (see `extractors.rs` for the rationale and the glob caveat).

pub use moatless_identity::adapter_apps::{
    app_id, app_key_from_transport_config, carry_over_env_and_load_config, dao, delete_adapter_app,
    linear_app_config, list_adapter_apps, slack_app_config, telegram_app_config, upsert_adapter_app,
    AdapterAppInput,
    AdapterAppSecretInput, LinearAppConfig, SlackAppConfig, StoredAdapterApp, TelegramAppConfig,
    DEFAULT_APP_KEY,
};
