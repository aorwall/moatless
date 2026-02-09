//! Feature flags endpoint — env-var based, no database.

use axum::Json;
use serde_json::{json, Value};

/// GET /api/v1/feature-flags
///
/// Returns all feature flags and their current values.
/// Flags are controlled by `FF_*` environment variables.
pub async fn get_feature_flags() -> Json<Value> {
    let mut flags = json!({
        "netlify_publish": env_flag("FF_NETLIFY_PUBLISH", false),
        "show_claude_settings": env_flag("FF_SHOW_CLAUDE_SETTINGS", true),
        "figma_clipboard_paste": env_flag("FF_FIGMA_CLIPBOARD_PASTE", true),
        "show_model_selector": env_flag("FF_SHOW_MODEL_SELECTOR", true),
        "gitlab_enabled": env_flag("FF_GITLAB_ENABLED", false),
        "slack_channel": env_flag("FF_SLACK_CHANNEL", false),
    });

    // Optional slack config
    if let Ok(team_id) = std::env::var("SLACK_TEAM_ID") {
        flags["slack"] = json!({ "teamId": team_id });
    }

    Json(flags)
}

/// Read a boolean feature flag from an environment variable.
fn env_flag(name: &str, default: bool) -> bool {
    std::env::var(name)
        .map(|v| v == "true" || v == "1")
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_flag_true_string() {
        let key = "TEST_FF_TRUE_STR_1";
        std::env::set_var(key, "true");
        assert!(env_flag(key, false));
        std::env::remove_var(key);
    }

    #[test]
    fn env_flag_one_string() {
        let key = "TEST_FF_ONE_STR_2";
        std::env::set_var(key, "1");
        assert!(env_flag(key, false));
        std::env::remove_var(key);
    }

    #[test]
    fn env_flag_false_string() {
        let key = "TEST_FF_FALSE_STR_3";
        std::env::set_var(key, "false");
        assert!(!env_flag(key, true));
        std::env::remove_var(key);
    }

    #[test]
    fn env_flag_unset_default_true() {
        let key = "TEST_FF_UNSET_TRUE_4";
        std::env::remove_var(key);
        assert!(env_flag(key, true));
    }

    #[test]
    fn env_flag_unset_default_false() {
        let key = "TEST_FF_UNSET_FALSE_5";
        std::env::remove_var(key);
        assert!(!env_flag(key, false));
    }
}
