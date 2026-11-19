//! Persisted shapes for the workspace band.
//!
//! The domain types live in `moatless_workspace_types::workspace`. What is here is the
//! shape the `workspaces` and `workspace_repos` tables actually have — the
//! run-config columns are JSONB, `kind` and `source` are text, and `deleted` is
//! the soft-delete flag the DAO filters on — plus the decode from one to the
//! other.

use chrono::{DateTime, Utc};
use moatless_core::db_enum::decode_required;
use tracing::error;

use moatless_types::access_control::Scope;

/// The workspace vocabulary a caller reaches through this module.
pub use moatless_workspace_types::workspace::{
    LanguageServer, ResourceConfig, ScriptConfig, Workspace, WorkspaceIcon, WorkspaceKind,
    WorkspaceRepo, WorkspaceSource,
};

// =============================================================================
// WorkspaceRow
// =============================================================================

/// Row from the `workspaces` table.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct WorkspaceRow {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub icon: Option<serde_json::Value>,
    /// Derived label from the `WorkspaceRepo` count: `blank`, `single`,
    /// `multi`.
    pub kind: String,
    // --- Run configuration ---
    pub docker_image: Option<String>,
    pub docker_access: Option<bool>,
    pub sandbox_service_account: Option<String>,
    pub system_prompt: Option<String>,
    pub setup_commands: Option<serde_json::Value>,
    pub env_vars: Option<serde_json::Value>,
    pub scripts: Option<serde_json::Value>,
    pub resources: Option<serde_json::Value>,
    pub language_servers: Option<serde_json::Value>,
    pub network_profile: Option<String>,
    pub node_selector: Option<serde_json::Value>,
    pub services: Option<serde_json::Value>,
    // --- Task defaults ---
    pub default_model: Option<String>,
    pub default_effort: Option<String>,
    pub linear_team: Option<String>,
    // --- Provenance ---
    pub source: String,
    pub source_repository_id: Option<String>,
    pub source_key: Option<String>,
    pub source_config_path: Option<String>,
    // --- Access control ---
    #[sqlx(try_from = "String")]
    pub scope: Scope,
    pub owner_user_id: Option<String>,
    pub run_as_user_id: Option<String>,
    pub created_by: Option<String>,
    /// Soft-delete flag. Git-origin tombstones stay visible in admin so an
    /// operator can restore them from git.
    pub deleted: Option<bool>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

impl TryFrom<WorkspaceRow> for Workspace {
    type Error = sqlx::Error;

    fn try_from(row: WorkspaceRow) -> Result<Self, Self::Error> {
        let id = row.id;
        Ok(Self {
            kind: decode_required("kind", row.kind)?,
            source: decode_required("source", row.source)?,
            setup_commands: decode_column(row.setup_commands, &id, "setup_commands"),
            env_vars: decode_column(row.env_vars, &id, "env_vars"),
            scripts: decode_column(row.scripts, &id, "scripts"),
            resources: decode_column(row.resources, &id, "resources"),
            language_servers: decode_column(row.language_servers, &id, "language_servers"),
            node_selector: decode_column(row.node_selector, &id, "node_selector"),
            services: decode_column(row.services, &id, "services"),
            icon: decode_column(row.icon, &id, "icon"),
            id,
            name: row.name,
            description: row.description,
            default_model: row.default_model,
            default_effort: row.default_effort,
            linear_team: row.linear_team,
            docker_image: row.docker_image,
            docker_access: row.docker_access,
            sandbox_service_account: row.sandbox_service_account,
            system_prompt: row.system_prompt,
            network_profile: row.network_profile,
            source_repository_id: row.source_repository_id,
            source_key: row.source_key,
            source_config_path: row.source_config_path,
            scope: row.scope,
            owner_user_id: row.owner_user_id,
            run_as_user_id: row.run_as_user_id,
            created_by: row.created_by,
            deleted: row.deleted.unwrap_or(false),
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

/// Decode one JSONB run-config column, logging what was dropped.
///
/// A column that fails to decode reads as unset, so one bad value cannot block
/// a task start — the sandbox falls back to deployment defaults, and the log is
/// what says why.
fn decode_column<T: serde::de::DeserializeOwned>(
    value: Option<serde_json::Value>,
    workspace_id: &str,
    column: &str,
) -> Option<T> {
    match serde_json::from_value(value?) {
        Ok(decoded) => Some(decoded),
        Err(e) => {
            error!(
                workspace_id,
                column,
                error = %e,
                "Failed to decode a workspace run-config column; reading it as unset"
            );
            None
        }
    }
}

/// Encode one run-config value into the JSONB column shape.
///
/// Strict, unlike the read side: a write that cannot be serialized fails rather
/// than silently persisting SQL `NULL` over the caller's configuration.
pub fn encode_column<T: serde::Serialize>(
    value: Option<&T>,
    column: &'static str,
) -> Result<Option<serde_json::Value>, sqlx::Error> {
    value
        .map(serde_json::to_value)
        .transpose()
        .map_err(|error| sqlx::Error::Encode(Box::new(ColumnEncodeError { column, error })))
}

/// A run-config value that could not be serialized into its JSONB column.
#[derive(Debug, thiserror::Error)]
#[error("failed to encode workspace column `{column}`: {error}")]
struct ColumnEncodeError {
    column: &'static str,
    error: serde_json::Error,
}

// =============================================================================
// WorkspaceRepoRow
// =============================================================================

/// Row from the `workspace_repos` table.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct WorkspaceRepoRow {
    pub id: String,
    pub workspace_id: String,
    pub repository_id: String,
    pub branch: Option<String>,
    pub mount_name: Option<String>,
    pub is_primary: bool,
    pub position: i32,
    pub created_at: Option<DateTime<Utc>>,
}

impl From<WorkspaceRepoRow> for WorkspaceRepo {
    fn from(row: WorkspaceRepoRow) -> Self {
        Self {
            id: row.id,
            workspace_id: row.workspace_id,
            repository_id: row.repository_id,
            branch: row.branch,
            mount_name: row.mount_name,
            is_primary: row.is_primary,
            position: row.position,
            created_at: row.created_at,
        }
    }
}

// =============================================================================
// ListedWorkspaceRow
// =============================================================================

/// Row from a widened listing: a workspace plus whether the caller may read it.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct ListedWorkspaceRow {
    #[sqlx(flatten)]
    pub workspace: WorkspaceRow,
    pub readable: bool,
}

/// A workspace in a widened listing, and whether the caller may actually
/// read it.
///
/// Unions what the read rule admits with what a thing the caller may see
/// merely references; the second half renders a group, not a grant. The
/// flag comes from the same access filter as the `WHERE`, so they can't drift.
#[derive(Debug, Clone)]
pub struct ListedWorkspace {
    pub workspace: Workspace,
    pub readable: bool,
}

impl TryFrom<ListedWorkspaceRow> for ListedWorkspace {
    type Error = sqlx::Error;

    fn try_from(row: ListedWorkspaceRow) -> Result<Self, Self::Error> {
        Ok(Self {
            workspace: Workspace::try_from(row.workspace)?,
            readable: row.readable,
        })
    }
}

/// Patch struct for partial workspace updates. Outer `Option` = "field present
/// in the patch"; inner `Option` = the nullable column value.
#[derive(Default)]
pub struct WorkspacePatch {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub icon: Option<Option<WorkspaceIcon>>,
    pub kind: Option<WorkspaceKind>,
    pub scope: Option<Scope>,
    pub owner_user_id: Option<Option<String>>,
    pub run_as_user_id: Option<Option<String>>,
    pub docker_image: Option<Option<String>>,
    pub docker_access: Option<Option<bool>>,
    pub sandbox_service_account: Option<Option<String>>,
    pub system_prompt: Option<Option<String>>,
    pub setup_commands: Option<Option<Vec<String>>>,
    pub env_vars: Option<Option<std::collections::HashMap<String, String>>>,
    pub scripts: Option<Option<Vec<ScriptConfig>>>,
    pub resources: Option<Option<ResourceConfig>>,
    pub language_servers: Option<Option<Vec<LanguageServer>>>,
    pub network_profile: Option<Option<String>>,
    pub node_selector: Option<Option<std::collections::BTreeMap<String, String>>>,
    pub services: Option<Option<Vec<String>>>,
    pub default_model: Option<Option<String>>,
    pub default_effort: Option<Option<String>>,
    pub linear_team: Option<Option<String>>,
    pub source: Option<WorkspaceSource>,
    pub source_repository_id: Option<Option<String>>,
    pub source_key: Option<Option<String>>,
    pub source_config_path: Option<Option<String>>,
}

/// What applying a [`WorkspacePatch`] under the caller's control scope did.
///
/// A plain `bool` conflated `NoFields` with `NoMatch`, and every caller
/// discarded it — so a caller who could read a Workspace but not control it got
/// `200` and the unchanged row back. Keep the two apart: only `NoMatch` is a
/// denial.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateOutcome {
    /// A row matched the caller's control scope and was written.
    Applied,
    /// The patch named no fields, so no statement ran and nothing is denied.
    NoFields,
    /// No row matched: missing, deleted, or outside the caller's control.
    ///
    /// A caller answers this with `NotFound` rather than `Forbidden`, so a
    /// denied write confirms no more than the read already did.
    NoMatch,
}
