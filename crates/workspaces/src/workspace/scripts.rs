//! Resolve the Scripts a Task can run.

use std::collections::BTreeMap;

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
}
