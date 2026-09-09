//! Team lifecycle and membership policy.

use super::{
    dao::TeamDao,
    error::TeamError,
    models::{Team, TeamCaller, TeamMember},
    schemas::{AddTeamMemberRequest, CreateTeamRequest, UpdateTeamRequest},
};

/// List teams with caller policy enforced.
pub async fn list_teams(teams: &dyn TeamDao, caller: &TeamCaller) -> Result<Vec<Team>, TeamError> {
    let teams = if caller.is_admin {
        teams.list_all().await?
    } else {
        teams.find_teams_for_user(&caller.user_id).await?
    };
    Ok(teams)
}

/// Create team with caller policy enforced.
pub async fn create_team(
    teams: &dyn TeamDao,
    caller: &TeamCaller,
    req: CreateTeamRequest,
) -> Result<Team, TeamError> {
    if teams.find_by_slug(&req.slug).await?.is_some() {
        return Err(TeamError::Conflict(format!(
            "Slug '{}' already taken",
            req.slug
        )));
    }

    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now();
    let team = Team {
        id: id.clone(),
        name: req.name,
        slug: req.slug,
        description: req.description,
        external_id: None,
        external_provider: None,
        is_default: false,
        settings: serde_json::json!({}),
        created_at: now,
        updated_at: now,
    };

    teams.insert(&team).await?;
    // Add creator as owner member
    teams.add_member(&id, &caller.user_id, "owner").await?;

    Ok(team)
}

/// Get team with caller policy enforced.
pub async fn get_team(
    teams: &dyn TeamDao,
    caller: &TeamCaller,
    team_id: String,
) -> Result<Team, TeamError> {
    let team = teams
        .find_by_id(&team_id)
        .await?
        .ok_or_else(|| TeamError::NotFound(team_id.clone()))?;

    // Non-admin must be a member
    if !caller.is_admin {
        let is_member = teams.is_member(&team_id, &caller.user_id).await?;
        if !is_member {
            return Err(TeamError::Forbidden("Not a team member".into()));
        }
    }

    Ok(team)
}

/// Update team with caller policy enforced.
pub async fn update_team(
    teams: &dyn TeamDao,
    caller: &TeamCaller,
    team_id: String,
    req: UpdateTeamRequest,
) -> Result<Team, TeamError> {
    let _existing = teams
        .find_by_id(&team_id)
        .await?
        .ok_or_else(|| TeamError::NotFound(team_id.clone()))?;
    ensure_team_manager(teams, caller, &team_id).await?;

    teams
        .update(&team_id, &req.name, req.description.as_deref())
        .await?;

    let updated = teams
        .find_by_id(&team_id)
        .await?
        .ok_or_else(|| TeamError::NotFound(team_id))?;

    Ok(updated)
}

/// Delete team with caller policy enforced.
pub async fn delete_team(
    teams: &dyn TeamDao,
    caller: &TeamCaller,
    team_id: String,
) -> Result<(), TeamError> {
    let _existing = teams
        .find_by_id(&team_id)
        .await?
        .ok_or_else(|| TeamError::NotFound(team_id.clone()))?;
    ensure_team_manager(teams, caller, &team_id).await?;

    teams.delete(&team_id).await?;
    Ok(())
}

/// List members with caller policy enforced.
pub async fn list_members(
    teams: &dyn TeamDao,
    caller: &TeamCaller,
    team_id: String,
) -> Result<Vec<TeamMember>, TeamError> {
    require_team_access(teams, caller, &team_id).await?;
    let members = teams.find_members(&team_id).await?;
    Ok(members)
}

/// Add member with caller policy enforced.
pub async fn add_member(
    teams: &dyn TeamDao,
    caller: &TeamCaller,
    team_id: String,
    req: AddTeamMemberRequest,
) -> Result<(), TeamError> {
    let _existing = teams
        .find_by_id(&team_id)
        .await?
        .ok_or_else(|| TeamError::NotFound(team_id.clone()))?;
    ensure_team_manager(teams, caller, &team_id).await?;

    let role = req.role.as_deref().unwrap_or("member");
    let user_id = req
        .user_id
        .parse::<uuid::Uuid>()
        .map_err(|_| TeamError::BadRequest("user_id must be a uuid".to_string()))?;
    teams.add_member(&team_id, &user_id, role).await?;
    Ok(())
}

/// Remove member with caller policy enforced.
pub async fn remove_member(
    teams: &dyn TeamDao,
    caller: &TeamCaller,
    team_id: String,
    user_id: String,
) -> Result<(), TeamError> {
    let _existing = teams
        .find_by_id(&team_id)
        .await?
        .ok_or_else(|| TeamError::NotFound(team_id.clone()))?;
    ensure_team_manager(teams, caller, &team_id).await?;

    let user_id = user_id
        .parse::<uuid::Uuid>()
        .map_err(|_| TeamError::NotFound(format!("User '{user_id}' not found")))?;
    teams.remove_member(&team_id, &user_id).await?;
    Ok(())
}

/// Verify the caller is an admin or an owner/admin member of the team.
async fn ensure_team_manager(
    teams: &dyn TeamDao,
    caller: &TeamCaller,
    team_id: &str,
) -> Result<(), TeamError> {
    if caller.is_admin {
        return Ok(());
    }
    let members = teams.find_members(team_id).await?;
    let caller_role = members
        .iter()
        .find(|m| m.user_id == caller.user_id)
        .map(|m| m.role.as_str());
    match caller_role {
        Some("owner") | Some("admin") => Ok(()),
        _ => Err(TeamError::Forbidden("Must be a team owner or admin".into())),
    }
}

/// Verify the caller is an admin or a member of the team.
async fn require_team_access(
    teams: &dyn TeamDao,
    caller: &TeamCaller,
    team_id: &str,
) -> Result<(), TeamError> {
    let _existing = teams
        .find_by_id(team_id)
        .await?
        .ok_or_else(|| TeamError::NotFound(team_id.to_string()))?;

    if !caller.is_admin {
        let is_member = teams.is_member(team_id, &caller.user_id).await?;
        if !is_member {
            return Err(TeamError::Forbidden("Not a team member".into()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::dao::MockTeamDao;
    use super::*;

    fn team() -> Team {
        Team {
            id: "team".into(),
            name: "Team".into(),
            slug: "team".into(),
            description: None,
            external_id: None,
            external_provider: None,
            is_default: false,
            settings: serde_json::json!({}),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[tokio::test]
    async fn non_manager_cannot_delete_through_the_operation() {
        let mut teams = MockTeamDao::new();
        teams.expect_find_by_id().returning(|_| Ok(Some(team())));
        teams.expect_find_members().returning(|_| Ok(vec![]));
        teams.expect_delete().never();
        let caller = TeamCaller {
            user_id: uuid::Uuid::nil(),
            is_admin: false,
        };
        assert!(matches!(
            delete_team(&teams, &caller, "team".into()).await,
            Err(TeamError::Forbidden(_))
        ));
    }

    #[tokio::test]
    async fn owner_can_delete_without_http_admission() {
        let mut teams = MockTeamDao::new();
        teams.expect_find_by_id().returning(|_| Ok(Some(team())));
        teams.expect_find_members().returning(|_| {
            Ok(vec![TeamMember {
                team_id: "team".into(),
                user_id: uuid::Uuid::nil(),
                role: "owner".into(),
                created_at: chrono::Utc::now(),
            }])
        });
        teams
            .expect_delete()
            .with(mockall::predicate::eq("team"))
            .once()
            .returning(|_| Ok(()));
        let caller = TeamCaller {
            user_id: uuid::Uuid::nil(),
            is_admin: false,
        };
        assert!(delete_team(&teams, &caller, "team".into()).await.is_ok());
    }
}
