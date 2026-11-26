//! Deployment feature-flag configuration.

/// Immutable snapshot of the deployment's reusable feature flags.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FeatureFlags {
    pub gitlab_enabled: bool,
    pub forgejo_enabled: bool,
    pub workspace_settings: bool,
}

impl FeatureFlags {
    /// Read every feature flag once through `lookup`.
    pub(crate) fn from_lookup(mut lookup: impl FnMut(&str) -> Option<String>) -> Self {
        Self {
            gitlab_enabled: flag_value(lookup("FF_GITLAB_ENABLED").as_deref(), false),
            forgejo_enabled: flag_value(lookup("FF_FORGEJO_ENABLED").as_deref(), false),
            workspace_settings: flag_value(lookup("FF_WORKSPACE_SETTINGS").as_deref(), true),
        }
    }
}

impl Default for FeatureFlags {
    fn default() -> Self {
        Self {
            gitlab_enabled: false,
            forgejo_enabled: false,
            workspace_settings: true,
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
                gitlab_enabled: false,
                forgejo_enabled: false,
                workspace_settings: true,
            }
        );
    }

    #[test]
    fn parses_configured_values_without_changing_the_process_environment() {
        let values = HashMap::from([
            ("FF_FORGEJO_ENABLED", "1"),
            ("FF_GITLAB_ENABLED", "TRUE"),
            ("FF_WORKSPACE_SETTINGS", "false"),
        ]);

        let flags = FeatureFlags::from_lookup(|name| values.get(name).map(ToString::to_string));

        assert_eq!(
            flags,
            FeatureFlags {
                gitlab_enabled: false,
                forgejo_enabled: true,
                workspace_settings: false,
            }
        );
    }
}
