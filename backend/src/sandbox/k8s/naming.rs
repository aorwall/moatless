//! Consolidated K8s resource name generation.
//!
//! All Kubernetes resource naming follows the same pattern:
//! sanitize the task ID, combine with a prefix, and truncate to 63 chars.

/// Sanitize a string for use in a Kubernetes resource name.
///
/// Lowercase, replace non-alphanumeric chars (except `-`) with `-`,
/// optionally truncate to `max_len` characters.
fn sanitize_k8s_name(input: &str, max_len: usize) -> String {
    input
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .take(max_len)
        .collect()
}

/// Truncate a name to the K8s 63-char limit.
fn truncate_k8s(name: String) -> String {
    if name.len() > 63 {
        name[..63].to_string()
    } else {
        name
    }
}

/// Generate a deterministic Kubernetes service name from a task ID.
///
/// Format: `moatless-ws-{sanitized_task_id}` (max 63 chars).
pub fn generate_service_name(task_id: &str) -> String {
    let sanitized = sanitize_k8s_name(task_id, 63);
    truncate_k8s(format!("moatless-ws-{sanitized}"))
}

/// Generate a unique PVC name for a task.
///
/// Format: `moatless-sandbox-{sanitized_task_id}-{base36_timestamp}` (max 63 chars).
pub fn generate_pvc_name(task_id: &str) -> String {
    let sanitized = sanitize_k8s_name(task_id, 30);
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let ts_str = radix_fmt(timestamp);
    truncate_k8s(format!("moatless-sandbox-{sanitized}-{ts_str}"))
}

/// Generate a unique pod name for a task.
///
/// Format: `moatless-{sanitized_repo}-{sanitized_task}-{hex_timestamp}` (max 63 chars).
pub fn generate_pod_name(repository_name: &str, task_id: &str) -> String {
    let sanitized_repo = sanitize_k8s_name(repository_name, 20);
    let sanitized_task = sanitize_k8s_name(task_id, 20);
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let ts_str = format!("{timestamp:x}");
    truncate_k8s(format!(
        "moatless-{sanitized_repo}-{sanitized_task}-{ts_str}"
    ))
}

/// Simple base36 formatter for compact timestamps.
fn radix_fmt(mut n: u128) -> String {
    if n == 0 {
        return "0".to_string();
    }
    let chars = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut result = Vec::new();
    while n > 0 {
        result.push(chars[(n % 36) as usize]);
        n /= 36;
    }
    result.reverse();
    String::from_utf8(result).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_k8s_name() {
        assert_eq!(sanitize_k8s_name("ABC_123", 63), "abc-123");
        assert_eq!(sanitize_k8s_name("hello.world", 63), "hello-world");
        assert_eq!(sanitize_k8s_name("a".repeat(100).as_str(), 20), "a".repeat(20));
    }

    #[test]
    fn test_generate_service_name() {
        assert_eq!(generate_service_name("abc123"), "moatless-ws-abc123");
        assert_eq!(generate_service_name("ABC123"), "moatless-ws-abc123");
        assert_eq!(generate_service_name("task_id"), "moatless-ws-task-id");
    }

    #[test]
    fn test_generate_service_name_truncation() {
        let long_id = "a".repeat(100);
        let name = generate_service_name(&long_id);
        assert!(name.len() <= 63);
    }

    #[test]
    fn test_generate_pvc_name_format() {
        let name = generate_pvc_name("task-123");
        assert!(name.starts_with("moatless-sandbox-task-123-"));
        assert!(name.len() <= 63);
    }

    #[test]
    fn test_generate_pod_name_format() {
        let name = generate_pod_name("my-repo", "task-123");
        assert!(name.starts_with("moatless-my-repo-task-123-"));
        assert!(name.len() <= 63);
    }

    #[test]
    fn test_generate_pod_name_truncation() {
        let name = generate_pod_name(&"a".repeat(50), &"b".repeat(50));
        assert!(name.len() <= 63);
    }

    #[test]
    fn test_radix_fmt() {
        assert_eq!(radix_fmt(0), "0");
        assert_eq!(radix_fmt(35), "z");
        assert_eq!(radix_fmt(36), "10");
    }
}
