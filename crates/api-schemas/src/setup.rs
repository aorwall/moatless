//! Environment-setup requirements reported alongside the session.
//!
//! The payload is deliberately machine-readable only — ids, statuses and
//! action targets. All user-facing copy lives in the frontend, keyed on
//! [`SetupRequirementId`], so wording changes don't need a backend deploy and
//! the backend never carries English strings.

use serde::{Deserialize, Serialize};

/// Environment-setup state for the current user.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SetupStatus {
    /// True when every `Required`-severity requirement is `Satisfied`.
    pub complete: bool,
    /// Every evaluated requirement, satisfied ones included, so the frontend
    /// can render a full checklist as well as a nag banner.
    #[serde(default)]
    pub requirements: Vec<SetupRequirement>,
}

/// One thing the environment needs before it can do its job.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SetupRequirement {
    pub id: SetupRequirementId,
    pub status: SetupRequirementStatus,
    pub severity: SetupSeverity,
    /// Remediation steps, most-preferred first. Empty when satisfied.
    /// Admin-only actions are omitted for non-admin callers, so the frontend
    /// never has to re-derive who may do what.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<SetupAction>,
}

/// A remediation step the frontend can offer. `target` is machine-readable:
/// a settings-section id, an in-app route, or an absolute URL, per `kind`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SetupAction {
    pub kind: SetupActionKind,
    pub target: String,
}

/// Which requirement this is. `Unknown` exists so an older client (notably the
/// `moat` CLI, which deserializes `SessionResponse`) doesn't hard-fail against
/// a newer server; the backend never constructs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SetupRequirementId {
    /// A GitHub token usable for reading repositories, at any scope the user
    /// can reach (their own, a team's, or the org-wide one).
    GithubToken,
    #[serde(other)]
    Unknown,
}

/// Tri-state because "configured but unusable" needs different copy from
/// "never configured": a `secret_ref` row whose secret is gone is `Broken`,
/// and telling that user to "connect GitHub" would be wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SetupRequirementStatus {
    Satisfied,
    Missing,
    Broken,
}

/// A closed vocabulary: a new value here is a real contract change the
/// frontend should fail to compile against rather than silently ignore.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SetupSeverity {
    /// Blocks core functionality. Surfaced persistently.
    Required,
    /// Worth doing, but the app works without it. Dismissible.
    Recommended,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SetupActionKind {
    /// `target` is a settings-dialog section id (e.g. `github`).
    OpenSettings,
    /// `target` is an in-app route the frontend maps to a typed link.
    AdminRoute,
    /// `target` is an absolute URL to open in a new tab.
    ExternalLink,
    #[serde(other)]
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_shape_is_camel_case_with_snake_case_enums() {
        let status = SetupStatus {
            complete: false,
            requirements: vec![SetupRequirement {
                id: SetupRequirementId::GithubToken,
                status: SetupRequirementStatus::Missing,
                severity: SetupSeverity::Required,
                actions: vec![SetupAction {
                    kind: SetupActionKind::OpenSettings,
                    target: "github".to_string(),
                }],
            }],
        };
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(json["complete"], false);
        assert_eq!(json["requirements"][0]["id"], "github_token");
        assert_eq!(json["requirements"][0]["status"], "missing");
        assert_eq!(json["requirements"][0]["severity"], "required");
        assert_eq!(
            json["requirements"][0]["actions"][0]["kind"],
            "open_settings"
        );
    }

    #[test]
    fn satisfied_requirements_omit_the_empty_actions_array() {
        let requirement = SetupRequirement {
            id: SetupRequirementId::GithubToken,
            status: SetupRequirementStatus::Satisfied,
            severity: SetupSeverity::Required,
            actions: vec![],
        };
        let json = serde_json::to_value(&requirement).unwrap();
        assert!(json.get("actions").is_none());
    }

    /// A newer server must not break an older `moat` CLI, which deserializes
    /// `SessionResponse` into these types.
    #[test]
    fn unknown_ids_and_action_kinds_deserialize_instead_of_failing() {
        let json = serde_json::json!({
            "complete": false,
            "requirements": [{
                "id": "some_future_requirement",
                "status": "missing",
                "severity": "required",
                "actions": [{"kind": "some_future_action", "target": "x"}]
            }]
        });
        let status: SetupStatus = serde_json::from_value(json).unwrap();
        assert_eq!(status.requirements[0].id, SetupRequirementId::Unknown);
        assert_eq!(
            status.requirements[0].actions[0].kind,
            SetupActionKind::Unknown
        );
    }

    #[test]
    fn a_missing_requirements_array_deserializes_as_empty() {
        let status: SetupStatus =
            serde_json::from_value(serde_json::json!({"complete": true})).unwrap();
        assert!(status.requirements.is_empty());
    }
}
