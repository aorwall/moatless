// This module compiles only for test builds (`#[cfg(test)]`) or downstream test
// tiers (`feature = "test-support"`), never a shipping build, so the seeder's
// `.expect()` calls are test scaffolding — allow them the way the crate's own
// `#[cfg(test)]` code is allowed (the crate-level allow is `test`-gated and does
// not cover the feature-gated build).
#![allow(clippy::expect_used, clippy::unwrap_used)]

//! Row factories for the workspaces band's PostgreSQL tiers.
//!
//! Gated `#[cfg(any(test, feature = "test-support"))]`, so it serves this
//! crate's own db-tests and, off `test-support`, the backend suites that seed a
//! repository or a workspace before exercising their own band.

use sqlx::PgPool;

use crate::repository::models::{Repository, RepositoryKind, RepositoryProvider};
use crate::workspace::models::{Workspace, WorkspaceRepo};

/// Seed the `teams` row that repository (and other team-scoped) rows FK to.
/// The factories below use `personal-{user_id}` as the team id; pass that value.
pub async fn seed_team(pool: &PgPool, team_id: &str) {
    sqlx::query(
        "INSERT INTO teams (id, name, slug) VALUES ($1, $2, $1) ON CONFLICT (id) DO NOTHING",
    )
    .bind(team_id)
    .bind(format!("Team {team_id}"))
    .execute(pool)
    .await
    .expect("seed team row");
}

/// A minimal user-scoped Repository.
pub fn make_repository_row(id: &str, user_id: &str) -> Repository {
    let now = chrono::Utc::now();
    Repository {
        id: id.to_string(),
        name: format!("repo-{id}"),
        kind: RepositoryKind::Git,
        remote_url: Some(format!("https://github.com/test/{id}.git")),
        default_branch: Some("main".to_string()),
        description: None,
        provider: Some(RepositoryProvider::Github),
        scope: moatless_types::access_control::Scope::User,
        owner_user_id: Some(user_id.to_string()),
        created_by: Some(user_id.to_string()),
        deleted: false,
        created_at: Some(now),
        updated_at: Some(now),
    }
}

/// A minimal user-scoped Workspace.
///
/// `kind` starts at `blank`; call `refresh_kind` after adding placements to
/// derive the real label. Run-config columns are left empty — placement and
/// ACL behaviour is what the store tests exercise.
pub fn make_workspace_row(id: &str, user_id: &str) -> Workspace {
    let now = chrono::Utc::now();
    Workspace {
        id: id.to_string(),
        name: format!("ws-{id}"),
        description: None,
        icon: None,
        default_model: None,
        default_effort: None,
        kind: moatless_types::workspace::WorkspaceKind::Blank,
        docker_image: None,
        docker_access: None,
        sandbox_service_account: None,
        system_prompt: None,
        setup_commands: None,
        env_vars: None,
        servers: None,
        scripts: None,
        resources: None,
        network_profile: None,
        source: moatless_types::workspace::WorkspaceSource::Manual,
        source_repository_id: None,
        source_key: None,
        source_config_path: None,
        run_as_user_id: None,
        scope: moatless_types::access_control::Scope::User,
        owner_user_id: Some(user_id.to_string()),
        created_by: Some(user_id.to_string()),
        deleted: false,
        created_at: Some(now),
        updated_at: Some(now),
    }
}

/// Decode a run-config JSON literal into the typed field a `Workspace` carries,
/// so a test can keep writing the shape the column stores.
pub fn run_config<T: serde::de::DeserializeOwned>(value: serde_json::Value) -> T {
    serde_json::from_value(value).expect("decode run-config literal")
}

/// A `workspace_repos` placement row.
pub fn make_workspace_repo_row(
    id: &str,
    workspace_id: &str,
    repository_id: &str,
    is_primary: bool,
    position: i32,
) -> WorkspaceRepo {
    WorkspaceRepo {
        id: id.to_string(),
        workspace_id: workspace_id.to_string(),
        repository_id: repository_id.to_string(),
        branch: None,
        mount_name: None,
        is_primary,
        position,
        created_at: Some(chrono::Utc::now()),
    }
}
