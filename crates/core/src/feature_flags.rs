//! Deployment feature-flag configuration.

/// Immutable snapshot of the deployment's reusable feature flags.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FeatureFlags {
    pub netlify_publish: bool,
    pub show_claude_settings: bool,
    pub show_codex_settings: bool,
    pub show_model_selector: bool,
    pub gitlab_enabled: bool,
}

impl FeatureFlags {
    /// Read every feature flag once from the process environment.
    pub fn from_env() -> Self {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    fn from_lookup(mut lookup: impl FnMut(&str) -> Option<String>) -> Self {
        Self {
            netlify_publish: flag_value(lookup("FF_NETLIFY_PUBLISH").as_deref(), false),
            show_claude_settings: flag_value(lookup("FF_SHOW_CLAUDE_SETTINGS").as_deref(), true),
            show_codex_settings: flag_value(lookup("FF_SHOW_CODEX_SETTINGS").as_deref(), true),
            show_model_selector: flag_value(lookup("FF_SHOW_MODEL_SELECTOR").as_deref(), true),
            gitlab_enabled: flag_value(lookup("FF_GITLAB_ENABLED").as_deref(), false),
        }
    }
}

impl Default for FeatureFlags {
    fn default() -> Self {
        Self {
            netlify_publish: false,
            show_claude_settings: true,
            show_codex_settings: true,
            show_model_selector: true,
            gitlab_enabled: false,
        }
    }
}

fn flag_value(value: Option<&str>, default: bool) -> bool {
    value.map_or(default, |value| value == "true" || value == "1")
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[test]
    fn defaults_match_the_deployment_contract() {
        assert_eq!(
            FeatureFlags::default(),
            FeatureFlags {
                netlify_publish: false,
                show_claude_settings: true,
                show_codex_settings: true,
                show_model_selector: true,
                gitlab_enabled: false,
            }
        );
    }

    #[test]
    fn parses_configured_values_without_changing_the_process_environment() {
        let values = HashMap::from([
            ("FF_NETLIFY_PUBLISH", "true"),
            ("FF_SHOW_CLAUDE_SETTINGS", "1"),
            ("FF_SHOW_CODEX_SETTINGS", "false"),
            ("FF_SHOW_MODEL_SELECTOR", "TRUE"),
        ]);

        let flags = FeatureFlags::from_lookup(|name| values.get(name).map(ToString::to_string));

        assert_eq!(
            flags,
            FeatureFlags {
                netlify_publish: true,
                show_claude_settings: true,
                show_codex_settings: false,
                show_model_selector: false,
                gitlab_enabled: false,
            }
        );
    }
}
