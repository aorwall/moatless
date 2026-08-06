//! Feature flags endpoint — env-var based, no database.

use axum::Json;
use serde::Serialize;

/// Slack workspace configuration.
#[derive(Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SlackConfig {
    pub team_id: String,
}

/// Response shape for GET /api/v1/feature-flags.
#[derive(Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FeatureFlagsResponse {
    pub netlify_publish: bool,
    pub show_claude_settings: bool,
    pub show_codex_settings: bool,
    pub show_model_selector: bool,
    pub gitlab_enabled: bool,
    /// Whether the settings dialog offers to pair a device.
    ///
    /// Not an `FF_*` variable of its own: pairing is the bootstrap half of the
    /// T3 compatibility routes, and a code is worth nothing on a deployment
    /// that does not mount them. So it reports whether *they* are on.
    pub device_pairing: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slack: Option<SlackConfig>,
}

/// GET /api/v1/feature-flags
///
/// Returns all feature flags and their current values.
/// Flags are controlled by `FF_*` environment variables.
#[cfg_attr(feature = "openapi", utoipa::path(
    get,
    path = "/api/v1/feature-flags",
    responses(
        (status = 200, description = "Feature flags", body = FeatureFlagsResponse),
    ),
    tag = "feature-flags",
))]
pub async fn get_feature_flags() -> Json<FeatureFlagsResponse> {
    let slack = std::env::var("SLACK_TEAM_ID")
        .ok()
        .map(|team_id| SlackConfig { team_id });

    Json(FeatureFlagsResponse {
        netlify_publish: env_flag("FF_NETLIFY_PUBLISH", false),
        show_claude_settings: env_flag("FF_SHOW_CLAUDE_SETTINGS", true),
        show_codex_settings: env_flag("FF_SHOW_CODEX_SETTINGS", true),
        show_model_selector: env_flag("FF_SHOW_MODEL_SELECTOR", true),
        gitlab_enabled: env_flag("FF_GITLAB_ENABLED", false),
        device_pairing: moatless_t3code::is_enabled(),
        slack,
    })
}

/// Read a boolean feature flag from an environment variable.
fn env_flag(name: &str, default: bool) -> bool {
    std::env::var(name).map_or(default, |v| v == "true" || v == "1")
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

    /// The pairing flag reads the T3 gate through the T3 module's own parser,
    /// which accepts spellings `env_flag` does not. Reading it here with
    /// `env_flag` instead would hide the settings section on a deployment whose
    /// routes are mounted.
    #[test]
    fn pairing_follows_the_gate_that_mounts_the_routes() {
        std::env::set_var("T3_UI_RPC_ENABLED", "on");
        assert!(moatless_t3code::is_enabled());
        assert!(
            !env_flag("T3_UI_RPC_ENABLED", false),
            "the reason we do not"
        );

        std::env::set_var("T3_UI_RPC_ENABLED", "false");
        assert!(!moatless_t3code::is_enabled());
        std::env::remove_var("T3_UI_RPC_ENABLED");
        assert!(!moatless_t3code::is_enabled());
    }
}
