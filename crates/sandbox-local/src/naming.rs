//! Per-task paths. The sandbox opens them directly, with no bind mount, so
//! each must exist before the process is spawned.

use std::path::PathBuf;

/// Per-task directory layout under `{root}/tasks/{task_id}/`.
#[allow(clippy::struct_field_names)]
pub struct TaskPaths {
    /// The task's own root directory — deleted wholesale on hard removal.
    pub base_dir: PathBuf,
    /// `WORKSPACE_DIR` — the cloned repository lives here.
    pub workspace_dir: PathBuf,
    /// `DATA_DIR` — shared scratch data.
    pub data_dir: PathBuf,
    /// `CLAUDE_CONFIG_DIR` — persisted Claude CLI session state.
    pub claude_config_dir: PathBuf,
    /// `HOME` — a real home directory the sandbox process runs as.
    pub home_dir: PathBuf,
    /// `RUNTIME_DIR` — where the sandbox's credential-broker socket binds.
    pub runtime_dir: PathBuf,
    /// `TMPDIR` — the sandbox wipes its temp dir on startup, so never the
    /// host's `/tmp`.
    pub tmp_dir: PathBuf,
}

impl TaskPaths {
    pub fn upload_dir(&self) -> PathBuf {
        self.workspace_dir.join("uploads")
    }

    pub fn codex_home_dir(&self) -> PathBuf {
        self.home_dir.join(".codex")
    }

    pub fn log_file(&self) -> PathBuf {
        self.base_dir.join("sandbox.log")
    }

    /// Directories that must exist before the sandbox process starts.
    pub fn dirs_to_create(&self) -> [&PathBuf; 6] {
        [
            &self.workspace_dir,
            &self.data_dir,
            &self.claude_config_dir,
            &self.home_dir,
            &self.runtime_dir,
            &self.tmp_dir,
        ]
    }
}

/// Generate the task-scoped root directory, rejecting path separators and
/// traversal components — the result feeds hard deletion.
///
/// Format: `{root}/tasks/{task_id}`
pub fn task_base_dir(root: &str, task_id: &str) -> Result<PathBuf, String> {
    let task_path = std::path::Path::new(task_id);
    let mut components = task_path.components();
    let valid = matches!(
        (components.next(), components.next()),
        (Some(std::path::Component::Normal(_)), None)
    );
    if !valid {
        return Err(format!("invalid task id for local sandbox path: {task_id}"));
    }

    let mut path = PathBuf::from(root);
    path.push("tasks");
    path.push(task_id);
    Ok(path)
}

/// Generate the full per-task path layout.
pub fn task_paths(root: &str, task_id: &str) -> Result<TaskPaths, String> {
    let base_dir = task_base_dir(root, task_id)?;
    Ok(TaskPaths {
        workspace_dir: base_dir.join("workspace"),
        data_dir: base_dir.join("data"),
        claude_config_dir: base_dir.join("claude-config"),
        home_dir: base_dir.join("home"),
        runtime_dir: base_dir.join("runtime"),
        tmp_dir: base_dir.join("tmp"),
        base_dir,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_path_rejects_traversal() {
        assert!(task_base_dir("/data", "../escape").is_err());
        assert!(task_base_dir("/data", "nested/task").is_err());
        assert_eq!(
            task_base_dir("/data", "task-123").unwrap(),
            PathBuf::from("/data/tasks/task-123")
        );
    }

    #[test]
    fn task_paths_layout() {
        let paths = task_paths("/root", "task-1").unwrap();
        assert_eq!(paths.base_dir, PathBuf::from("/root/tasks/task-1"));
        assert_eq!(
            paths.workspace_dir,
            PathBuf::from("/root/tasks/task-1/workspace")
        );
        assert_eq!(
            paths.upload_dir(),
            PathBuf::from("/root/tasks/task-1/workspace/uploads")
        );
        assert_eq!(
            paths.codex_home_dir(),
            PathBuf::from("/root/tasks/task-1/home/.codex")
        );
    }
}
