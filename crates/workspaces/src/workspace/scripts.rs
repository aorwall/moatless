//! Authorize script replacement and preserve environment values hidden from clients.

use moatless_types::access_control::ScopeContext;
use moatless_types::workspace::ScriptConfig;

use super::dao::WorkspaceDao;
use super::error::WorkspaceScriptsError;

/// Replace readable manual scripts while retaining environments by script name.
pub async fn replace_scripts_preserving_env(
    workspaces: &dyn WorkspaceDao,
    viewer: &ScopeContext,
    workspace_id: &str,
    submitted: &[ScriptConfig],
) -> Result<(), WorkspaceScriptsError> {
    let target = workspaces
        .scripts_write_target(viewer, workspace_id)
        .await?
        .ok_or(WorkspaceScriptsError::NotFound)?;
    if target.source == "git" {
        return Err(WorkspaceScriptsError::GitSynced);
    }
    let mut names = std::collections::HashSet::new();
    let mut next = submitted.to_vec();
    for script in &mut next {
        if !names.insert(script.name.clone()) {
            return Err(WorkspaceScriptsError::Invalid(format!(
                "Two scripts share the name `{}`.",
                script.name
            )));
        }
        script.env_vars = target
            .scripts
            .iter()
            .find(|stored| stored.name == script.name)
            .and_then(|stored| stored.env_vars.clone());
    }
    if !workspaces
        .replace_scripts(viewer, workspace_id, &next)
        .await?
    {
        return Err(WorkspaceScriptsError::NotFound);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::dao::{MockWorkspaceDao, WorkspaceScriptsTarget};
    use super::*;

    fn viewer() -> ScopeContext {
        ScopeContext {
            user_id: "viewer".to_string(),
            is_admin: false,
        }
    }

    #[tokio::test]
    async fn replacement_preserves_only_existing_named_environments() {
        let mut dao = MockWorkspaceDao::new();
        dao.expect_scripts_write_target().returning(|_, _| {
            Ok(Some(WorkspaceScriptsTarget {
                source: "manual".to_string(),
                scripts: vec![ScriptConfig {
                    name: "build".to_string(),
                    env_vars: Some([("KEEP".to_string(), "yes".to_string())].into()),
                    ..Default::default()
                }],
            }))
        });
        dao.expect_replace_scripts()
            .once()
            .withf(|_, id, scripts| {
                id == "ws"
                    && scripts[0]
                        .env_vars
                        .as_ref()
                        .is_some_and(|env| env.get("KEEP").is_some_and(|v| v == "yes"))
                    && scripts[1].env_vars.is_none()
            })
            .returning(|_, _, _| Ok(true));
        let submitted = [
            ScriptConfig {
                name: "build".to_string(),
                command: "new command".to_string(),
                ..Default::default()
            },
            ScriptConfig {
                name: "new".to_string(),
                env_vars: Some([("INJECT".to_string(), "no".to_string())].into()),
                ..Default::default()
            },
        ];
        replace_scripts_preserving_env(&dao, &viewer(), "ws", &submitted)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn git_owned_scripts_are_refused_before_writing() {
        let mut dao = MockWorkspaceDao::new();
        dao.expect_scripts_write_target().returning(|_, _| {
            Ok(Some(WorkspaceScriptsTarget {
                source: "git".to_string(),
                scripts: vec![],
            }))
        });
        assert!(matches!(
            replace_scripts_preserving_env(&dao, &viewer(), "ws", &[]).await,
            Err(WorkspaceScriptsError::GitSynced)
        ));
    }

    #[tokio::test]
    async fn a_rejected_guarded_write_is_hidden_as_missing() {
        let mut dao = MockWorkspaceDao::new();
        dao.expect_scripts_write_target().returning(|_, _| {
            Ok(Some(WorkspaceScriptsTarget {
                source: "manual".to_string(),
                scripts: vec![],
            }))
        });
        dao.expect_replace_scripts().returning(|_, _, _| Ok(false));
        assert!(matches!(
            replace_scripts_preserving_env(&dao, &viewer(), "ws", &[]).await,
            Err(WorkspaceScriptsError::NotFound)
        ));
    }
}
