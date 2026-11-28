//! Resolve the Scripts a Task can run.

use std::collections::{BTreeMap, HashMap};

use moatless_workspace_types::workspace::ScriptConfig;

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
/// shared name.
///
/// Workspace Scripts first in stored order, then the Task's by name. A
/// shadowing Task Script keeps the shadowed one's `env_vars` under its own,
/// since it cannot declare any and would otherwise run without them.
pub fn merge(
    workspace: Vec<ScriptConfig>,
    mut own: BTreeMap<String, ScriptConfig>,
) -> Vec<ResolvedScript> {
    let mut scripts = Vec::with_capacity(workspace.len() + own.len());
    for config in workspace {
        match own.get_mut(&config.name) {
            Some(task) => inherit_env_vars(task, config.env_vars),
            None => scripts.push(ResolvedScript {
                config,
                declared_by: ScriptOrigin::Workspace,
            }),
        }
    }
    scripts.extend(own.into_values().map(|config| ResolvedScript {
        config,
        declared_by: ScriptOrigin::Task,
    }));
    scripts
}

/// `inherited` under `task`'s own `env_vars`, the Task's value winning a key.
fn inherit_env_vars(task: &mut ScriptConfig, inherited: Option<HashMap<String, String>>) {
    let Some(mut env) = inherited else {
        return;
    };
    env.extend(task.env_vars.take().unwrap_or_default());
    task.env_vars = Some(env);
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// Redeclaring `dev` on a Task to change its command must not drop the
    /// workspace's env: vite then proxied without the dev-auth header.
    #[test]
    fn a_task_script_inherits_the_shadowed_scripts_env_vars_under_its_own() {
        let mut workspace = config("dev", "bun dev");
        workspace.env_vars = Some(HashMap::from([
            ("DEV_AUTH_ENABLED".to_string(), "true".to_string()),
            ("PORT".to_string(), "5173".to_string()),
        ]));
        let mut task = config("dev", "bun dev --port 5174");
        task.env_vars = Some(HashMap::from([("PORT".to_string(), "5174".to_string())]));

        let merged = merge(vec![workspace], BTreeMap::from([("dev".to_string(), task)]));

        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].declared_by, ScriptOrigin::Task);
        assert_eq!(merged[0].config.command, "bun dev --port 5174");
        let env = merged[0].config.env_vars.as_ref().expect("env inherited");
        assert_eq!(env["DEV_AUTH_ENABLED"], "true");
        assert_eq!(env["PORT"], "5174", "the task's own value wins");
    }

    /// A Task Script that shadows nothing keeps the env it was given.
    #[test]
    fn a_task_script_with_no_namesake_keeps_its_env_unchanged() {
        let merged = merge(
            vec![config("dev", "bun dev")],
            BTreeMap::from([("demo".to_string(), config("demo", "serve"))]),
        );
        let demo = merged
            .iter()
            .find(|s| s.config.name == "demo")
            .expect("demo");
        assert_eq!(demo.config.env_vars, None);
    }
}
