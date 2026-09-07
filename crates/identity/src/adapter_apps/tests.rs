use super::{DEFAULT_APP_KEY, app_key_from_transport_config};

#[test]
fn transport_config_app_key_defaults_when_absent_or_blank() {
    assert_eq!(
        app_key_from_transport_config(&serde_json::json!({})),
        DEFAULT_APP_KEY
    );
    assert_eq!(
        app_key_from_transport_config(&serde_json::json!({ "app_key": "   " })),
        DEFAULT_APP_KEY
    );
    assert_eq!(
        app_key_from_transport_config(&serde_json::json!({ "app_key": "enterprise" })),
        "enterprise"
    );
}
