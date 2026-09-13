//! Feature-flag HTTP presentation and OpenAPI registration.

use axum::{Json, extract::State};
use moatless_core::feature_flags::FeatureFlags;
use serde::Serialize;
use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

/// Response shape for GET /api/v1/feature-flags.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, utoipa::ToSchema)]
pub struct FeatureFlagsResponse {
    pub netlify_publish: bool,
    pub show_claude_settings: bool,
    pub show_codex_settings: bool,
    pub show_model_selector: bool,
    pub gitlab_enabled: bool,
    /// Whether the deployment offers Forgejo instances, connections and repos.
    pub forgejo_enabled: bool,
    /// Whether a T3 project's settings page edits the Workspace behind it.
    ///
    /// On, the page gains the Workspace's own description, repositories, image
    /// and defaults, and drops the three rows a Moatless Workspace does not
    /// have (`defaultThreadEnvMode`, auto-pull, agent browser access).
    pub workspace_settings: bool,
    /// Whether the settings dialog offers to pair a device.
    ///
    /// Not an `FF_*` variable of its own: pairing is the bootstrap half of the
    /// T3 compatibility routes, and a code is worth nothing on a deployment
    /// that does not mount them. So it reports whether *they* are on.
    pub device_pairing: bool,
}

impl FeatureFlagsResponse {
    fn new(flags: FeatureFlags, device_pairing: bool) -> Self {
        Self {
            netlify_publish: flags.netlify_publish,
            show_claude_settings: flags.show_claude_settings,
            show_codex_settings: flags.show_codex_settings,
            show_model_selector: flags.show_model_selector,
            gitlab_enabled: flags.gitlab_enabled,
            forgejo_enabled: flags.forgejo_enabled,
            workspace_settings: flags.workspace_settings,
            device_pairing,
        }
    }
}

/// GET /api/v1/feature-flags
///
/// Returns all feature flags and their current values.
/// Flags are controlled by `FF_*` environment variables.
#[utoipa::path(
    get,
    path = "/api/v1/feature-flags",
    responses(
        (status = 200, description = "Feature flags", body = FeatureFlagsResponse),
    ),
    tag = "feature-flags",
)]
pub async fn get_feature_flags(
    State(response): State<FeatureFlagsResponse>,
) -> Json<FeatureFlagsResponse> {
    Json(response)
}

#[derive(OpenApi)]
#[openapi(paths(get_feature_flags), components(schemas(FeatureFlagsResponse)))]
pub struct FeatureFlagsApiDoc;

/// Build the feature-flags router from the deployment snapshot and capabilities.
pub fn feature_flags_router(
    flags: FeatureFlags,
    device_pairing: bool,
) -> (axum::Router, utoipa::openapi::OpenApi) {
    let response = FeatureFlagsResponse::new(flags, device_pairing);
    OpenApiRouter::with_openapi(FeatureFlagsApiDoc::openapi())
        .routes(routes!(get_feature_flags))
        .with_state(response)
        .split_for_parts()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flags() -> FeatureFlags {
        FeatureFlags {
            netlify_publish: true,
            show_claude_settings: false,
            show_codex_settings: true,
            show_model_selector: false,
            gitlab_enabled: true,
            forgejo_enabled: true,
            workspace_settings: true,
        }
    }

    #[test]
    fn response_contains_the_snapshot() {
        let response = FeatureFlagsResponse::new(flags(), false);

        assert_eq!(
            response,
            FeatureFlagsResponse {
                netlify_publish: true,
                show_claude_settings: false,
                show_codex_settings: true,
                show_model_selector: false,
                gitlab_enabled: true,
                forgejo_enabled: true,
                workspace_settings: true,
                device_pairing: false,
            }
        );
    }

    #[test]
    fn response_reports_mounted_device_pairing() {
        let response = FeatureFlagsResponse::new(flags(), true);

        assert!(response.device_pairing);
    }
}
