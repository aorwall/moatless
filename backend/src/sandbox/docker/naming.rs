//! Docker container and volume path name generation.
//!
//! Container names are deterministic from the task ID, enabling lookup
//! without any database or external state. Docker allows container names
//! matching `[a-zA-Z0-9][a-zA-Z0-9_.-]`.

use std::path::PathBuf;

/// Maximum Docker container name length.
const MAX_CONTAINER_NAME_LEN: usize = 128;

/// Sanitize a string for use in a Docker container name.
///
/// Lowercase, replace non-alphanumeric chars (except `-`, `_`, `.`) with `-`,
/// truncate to `max_len` characters.
fn sanitize_docker_name(input: &str, max_len: usize) -> String {
    input
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .take(max_len)
        .collect()
}

/// Generate a deterministic Docker container name from a task ID.
///
/// Format: `moatless-sandbox-{sanitized_task_id}` (max 128 chars).
pub fn generate_container_name(task_id: &str) -> String {
    let prefix = "moatless-sandbox-";
    let max_id_len = MAX_CONTAINER_NAME_LEN - prefix.len();
    let sanitized = sanitize_docker_name(task_id, max_id_len);
    format!("{prefix}{sanitized}")
}

/// Generate a deterministic Docker container name for a server sidecar.
///
/// Format: `moatless-server-{sanitized_task_id}-{sanitized_server_name}` (max 128 chars).
pub fn generate_server_container_name(task_id: &str, server_name: &str) -> String {
    let prefix = "moatless-server-";
    let separator = "-";
    // Available chars = max - prefix - separator
    let available = MAX_CONTAINER_NAME_LEN - prefix.len() - separator.len();
    // Split available space: give 2/3 to task_id, 1/3 to server_name (min 10)
    let name_budget = available.min(server_name.len()).max(10.min(available));
    let task_budget = available.saturating_sub(name_budget);
    let sanitized_task = sanitize_docker_name(task_id, task_budget);
    let sanitized_name = sanitize_docker_name(server_name, name_budget);
    format!("{prefix}{sanitized_task}{separator}{sanitized_name}")
}

/// Generate the host volume path for a task's workspace.
///
/// Format: `{data_dir}/tasks/{task_id}/repo`
pub fn generate_workspace_host_path(data_dir: &str, task_id: &str) -> String {
    let mut path = PathBuf::from(data_dir);
    path.push("tasks");
    path.push(task_id);
    path.push("repo");
    path.to_string_lossy().to_string()
}

/// Generate the host volume path for a task's shared data directory.
///
/// Used for the `.workspace-ready` sentinel file that server containers
/// poll before starting. Format: `{data_dir}/tasks/{task_id}/data/`
pub fn generate_data_host_path(data_dir: &str, task_id: &str) -> String {
    let mut path = PathBuf::from(data_dir);
    path.push("tasks");
    path.push(task_id);
    path.push("data");
    path.to_string_lossy().to_string()
}

/// Generate the host volume path for a task's Claude config directory.
///
/// Persists Claude CLI session data (`CLAUDE_CONFIG_DIR`) across container
/// stop/start and recreation cycles. Format: `{data_dir}/tasks/{task_id}/claude-config`
pub fn generate_claude_config_host_path(data_dir: &str, task_id: &str) -> String {
    let mut path = PathBuf::from(data_dir);
    path.push("tasks");
    path.push(task_id);
    path.push("claude-config");
    path.to_string_lossy().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_name_basic() {
        assert_eq!(
            generate_container_name("abc123"),
            "moatless-sandbox-abc123"
        );
    }

    #[test]
    fn container_name_sanitizes_uppercase() {
        assert_eq!(
            generate_container_name("ABC123"),
            "moatless-sandbox-abc123"
        );
    }

    #[test]
    fn container_name_sanitizes_special_chars() {
        assert_eq!(
            generate_container_name("task/id@here"),
            "moatless-sandbox-task-id-here"
        );
    }

    #[test]
    fn container_name_preserves_underscores_and_dots() {
        assert_eq!(
            generate_container_name("task_id.v2"),
            "moatless-sandbox-task_id.v2"
        );
    }

    #[test]
    fn container_name_truncation() {
        let long_id = "a".repeat(200);
        let name = generate_container_name(&long_id);
        assert!(name.len() <= MAX_CONTAINER_NAME_LEN);
        assert!(name.starts_with("moatless-sandbox-"));
    }

    #[test]
    fn workspace_path_basic() {
        assert_eq!(
            generate_workspace_host_path("/data", "task-123"),
            "/data/tasks/task-123/repo"
        );
    }

    #[test]
    fn workspace_path_trailing_slash() {
        // PathBuf normalises, so trailing slash on data_dir is handled
        let path = generate_workspace_host_path("/data/", "task-123");
        assert!(path.contains("tasks"));
        assert!(path.contains("task-123"));
    }

    #[test]
    fn server_container_name_basic() {
        assert_eq!(
            generate_server_container_name("task-123", "preview"),
            "moatless-server-task-123-preview"
        );
    }

    #[test]
    fn server_container_name_sanitizes() {
        assert_eq!(
            generate_server_container_name("TASK/123", "Preview Server"),
            "moatless-server-task-123-preview-server"
        );
    }

    #[test]
    fn server_container_name_truncation() {
        let long_id = "a".repeat(200);
        let name = generate_server_container_name(&long_id, "preview");
        assert!(name.len() <= MAX_CONTAINER_NAME_LEN);
        assert!(name.starts_with("moatless-server-"));
    }
}
