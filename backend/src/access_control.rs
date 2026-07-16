//! Database-backed helpers for the canonical scoped-resource policy.

use chrono::Utc;

use crate::auth::models::UserRole;
use crate::auth::users::User;
use crate::extractors::AuthUser;
use crate::state::AppState;
use crate::team::models::Team;
use moatless_backend_core::scope::{Scope, ScopeContext, ScopedResource, TeamRole};

pub const SYSTEM_TEAM_ID: &str = "system";

#[derive(Debug, thiserror::Error)]
pub enum AccessControlError {
    #[error("Bad request: {0}")]
    BadRequest(String),
    #[error("Forbidden: {0}")]
    Forbidden(String),
    #[error("Database error: {0}")]
    Db(#[from] sqlx::Error),
}

#[derive(Debug, Clone)]
pub struct ResolvedScope {
    pub scope: Scope,
    pub team_id: String,
    pub owner_user_id: Option<String>,
}

pub async fn resolve_scope_for_create(
    state: &AppState,
    auth: &AuthUser,
    requested_scope: Option<Scope>,
    requested_team_id: Option<String>,
) -> Result<ResolvedScope, AccessControlError> {
    let scope = requested_scope.unwrap_or(Scope::User);
    let team_id = match scope {
        Scope::Global => ensure_system_team(state).await?,
        Scope::Team | Scope::User => resolve_team_id(state, auth, requested_team_id).await?,
    };
    let owner_user_id = (scope == Scope::User).then(|| auth.user_id.clone());
    let role = team_role_for_user(state, auth, &team_id).await?;

    if !auth
        .scope_context()
        .can_create_scope(scope, owner_user_id.as_deref(), role)
    {
        return Err(AccessControlError::Forbidden(format!(
            "Cannot create {scope} scoped resources for team {team_id}"
        )));
    }

    Ok(ResolvedScope {
        scope,
        team_id,
        owner_user_id,
    })
}

pub async fn ensure_can_write_resource(
    state: &AppState,
    auth: &AuthUser,
    resource: &ScopedResource,
    resource_name: &str,
) -> Result<(), AccessControlError> {
    let role = resource_team_role(state, auth, resource).await?;
    if auth.scope_context().can_write(resource, role) {
        Ok(())
    } else {
        Err(AccessControlError::Forbidden(format!(
            "No write access to {resource_name}"
        )))
    }
}

pub async fn ensure_can_control_task(
    state: &AppState,
    auth: &AuthUser,
    task: &ScopedResource,
) -> Result<(), AccessControlError> {
    let role = resource_team_role(state, auth, task).await?;
    if auth.scope_context().can_control_task(task, role) {
        Ok(())
    } else {
        Err(AccessControlError::Forbidden(
            "No control access to task".to_string(),
        ))
    }
}

pub fn ensure_can_use_repository(
    ctx: &ScopeContext,
    repository: &ScopedResource,
) -> Result<(), AccessControlError> {
    if ctx.can_use_repository(repository) {
        Ok(())
    } else {
        Err(AccessControlError::Forbidden(
            "No access to repository".to_string(),
        ))
    }
}

pub async fn ensure_team_manager(
    state: &AppState,
    auth: &AuthUser,
    team_id: &str,
) -> Result<(), AccessControlError> {
    if auth.scope_context().is_admin {
        return Ok(());
    }
    match team_role_for_user(state, auth, team_id).await? {
        Some(role) if role.can_manage() => Ok(()),
        _ => Err(AccessControlError::Forbidden(format!(
            "Admin role required for team {team_id}"
        ))),
    }
}

async fn resource_team_role(
    state: &AppState,
    auth: &AuthUser,
    resource: &ScopedResource,
) -> Result<Option<TeamRole>, AccessControlError> {
    let Some(team_id) = resource.team_id.as_deref() else {
        return Ok(None);
    };
    team_role_for_user(state, auth, team_id).await
}

pub async fn team_role_for_user(
    state: &AppState,
    auth: &AuthUser,
    team_id: &str,
) -> Result<Option<TeamRole>, AccessControlError> {
    if auth.scope_context().is_admin {
        return Ok(Some(TeamRole::Owner));
    }

    let members = state.teams.find_members(team_id).await?;
    Ok(members
        .into_iter()
        .find(|m| m.user_id == auth.user_id)
        .and_then(|m| TeamRole::try_from(m.role.as_str()).ok()))
}

async fn resolve_team_id(
    state: &AppState,
    auth: &AuthUser,
    requested_team_id: Option<String>,
) -> Result<String, AccessControlError> {
    if let Some(team_id) = requested_team_id {
        if auth.scope_context().is_admin || state.teams.is_member(&team_id, &auth.user_id).await? {
            return Ok(team_id);
        }
        return Err(AccessControlError::Forbidden(format!(
            "No access to team {team_id}"
        )));
    }

    let teams = state.teams.find_teams_for_user(&auth.user_id).await?;
    match teams.as_slice() {
        [team] => Ok(team.id.clone()),
        [] => ensure_personal_team(state, auth).await,
        _ => Err(AccessControlError::BadRequest(
            "teamId is required when the user belongs to multiple teams".to_string(),
        )),
    }
}

async fn ensure_system_team(state: &AppState) -> Result<String, AccessControlError> {
    if state.teams.find_by_id(SYSTEM_TEAM_ID).await?.is_some() {
        return Ok(SYSTEM_TEAM_ID.to_string());
    }
    let now = Utc::now();
    let team = Team {
        id: SYSTEM_TEAM_ID.to_string(),
        name: "System".to_string(),
        slug: "system".to_string(),
        description: Some("Reserved team for global system resources".to_string()),
        external_id: None,
        external_provider: None,
        is_default: false,
        settings: serde_json::json!({}),
        created_at: now,
        updated_at: now,
    };
    state.teams.insert_if_absent(&team).await?;
    Ok(SYSTEM_TEAM_ID.to_string())
}

async fn ensure_personal_team(
    state: &AppState,
    auth: &AuthUser,
) -> Result<String, AccessControlError> {
    ensure_user_row(state, auth).await?;

    let team_id = format!("personal-{}", auth.user_id);
    if state.teams.find_by_id(&team_id).await?.is_none() {
        let now = Utc::now();
        let slug = format!("personal-{}", slugify(&auth.user_id));
        let team = Team {
            id: team_id.clone(),
            name: format!("{} Personal", auth.login),
            slug,
            description: Some("Personal workspace".to_string()),
            external_id: None,
            external_provider: None,
            is_default: false,
            settings: serde_json::json!({}),
            created_at: now,
            updated_at: now,
        };
        state.teams.insert_if_absent(&team).await?;
    }

    state
        .teams
        .add_member(&team_id, &auth.user_id, "owner")
        .await?;
    Ok(team_id)
}

async fn ensure_user_row(state: &AppState, auth: &AuthUser) -> Result<(), AccessControlError> {
    if state.users.get_user(&auth.user_id).await?.is_some() {
        return Ok(());
    }
    let now = Utc::now();
    let role = auth
        .role
        .as_deref()
        .and_then(|role| UserRole::try_from(role).ok())
        .unwrap_or(UserRole::User);
    let user = User {
        id: auth.user_id.clone(),
        login: auth.login.clone(),
        email: None,
        name: None,
        avatar_url: None,
        provider: auth.provider.clone().unwrap_or_else(|| "local".to_string()),
        role,
        created_at: now,
        last_login_at: now,
        settings: serde_json::json!({}),
        is_bot: false,
    };
    state.users.upsert_user(&user).await?;
    Ok(())
}

fn slugify(value: &str) -> String {
    let mut slug = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::slugify;

    #[test]
    fn slugify_keeps_slug_url_safe() {
        assert_eq!(slugify("User.Name@example.com"), "user-name-example-com");
        assert_eq!(slugify("--abc--"), "abc");
    }
}
