//! Resolve the Scripts a Task can run, and authorize script replacement while
//! preserving environment values hidden from clients.

use std::collections::BTreeMap;

use moatless_types::access_control::ScopeContext;
use moatless_workspace_types::workspace::ScriptConfig;

use super::error::WorkspaceScriptsError;
use super::operations::Workspaces;

/// Where a resolved Script was declared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptOrigin {
    /// On the Workspace, so every Task under it can run it.
    Workspace,
    /// On one Task, stored in its Sandbox; it runs on that Task alone.
    Task,
}

/// One Script a Task can run, and where it was declared.
#[derive(Debug, Clone)]
pub struct ResolvedScript {
    pub config: ScriptConfig,
    pub declared_by: ScriptOrigin,
}

/// The Workspace's Scripts with the Task's own over them, the Task's winning a
/// shared name as the more specific of the two. Workspace Scripts come first,
/// in their stored order, then the Task's by name.
pub fn merge(
    workspace: Vec<ScriptConfig>,
    own: BTreeMap<String, ScriptConfig>,
) -> Vec<ResolvedScript> {
    let mut scripts: Vec<ResolvedScript> = workspace
        .into_iter()
        .filter(|script| !own.contains_key(&script.name))
        .map(|config| ResolvedScript {
            config,
            declared_by: ScriptOrigin::Workspace,
        })
        .collect();
    scripts.extend(own.into_values().map(|config| ResolvedScript {
        config,
        declared_by: ScriptOrigin::Task,
    }));
    scripts
}

/// Replace readable manual scripts while retaining environments by script name.
pub async fn replace_scripts_preserving_env(
    workspaces: &Workspaces,
    viewer: &ScopeContext,
    workspace_id: &str,
    submitted: &[ScriptConfig],
) -> Result<(), WorkspaceScriptsError> {
    let workspaces = workspaces.store();
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
    use super::super::dao::MockWorkspaceDao;
    use super::super::models::WorkspaceScriptsTarget;
    use super::*;
    use std::sync::Arc;

    /// The operation handle over a steered store.
    fn over(dao: MockWorkspaceDao) -> Workspaces {
        Workspaces::new(Arc::new(dao), Default::default())
    }

    fn viewer() -> ScopeContext {
        ScopeContext::user("viewer")
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
        replace_scripts_preserving_env(&over(dao), &viewer(), "ws", &submitted)
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
            replace_scripts_preserving_env(&over(dao), &viewer(), "ws", &[]).await,
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
            replace_scripts_preserving_env(&over(dao), &viewer(), "ws", &[]).await,
            Err(WorkspaceScriptsError::NotFound)
        ));
    }

    fn config(name: &str, command: &str) -> ScriptConfig {
        ScriptConfig {
            name: name.to_string(),
            command: command.to_string(),
            ..Default::default()
        }
    }

    /// Declaring `demo` on one Task is how a Workspace Script is overridden for
    /// that Task alone, so the Workspace's copy must not be listed beside it or
    /// `run demo` has two candidates.
    #[test]
    fn a_task_script_shadows_the_workspace_script_of_the_same_name() {
        let own = BTreeMap::from([("demo".to_string(), config("demo", "python3 -m http.server"))]);
        let merged = merge(
            vec![config("dev", "bun dev"), config("demo", "bun demo")],
            own,
        );

        let demo: Vec<_> = merged
            .iter()
            .filter(|script| script.config.name == "demo")
            .collect();
        assert_eq!(demo.len(), 1, "one `demo`, not both");
        assert_eq!(demo[0].config.command, "python3 -m http.server");
        assert_eq!(demo[0].declared_by, ScriptOrigin::Task);

        let dev = merged
            .iter()
            .find(|script| script.config.name == "dev")
            .expect("the workspace's other script survives");
        assert_eq!(dev.declared_by, ScriptOrigin::Workspace);
    }

    /// A Task that declared nothing still runs its Workspace's Scripts, all
    /// reported as the Workspace's.
    #[test]
    fn a_task_with_no_scripts_of_its_own_lists_its_workspace_unchanged() {
        let merged = merge(vec![config("dev", "bun dev")], BTreeMap::new());
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].declared_by, ScriptOrigin::Workspace);
    }
}
