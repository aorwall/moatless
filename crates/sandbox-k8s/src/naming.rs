//! Consolidated K8s resource name generation.
//!
//! All Kubernetes resource naming follows the same pattern: sanitize the task
//! ID, combine with a prefix, and truncate to 63 chars.

/// Sanitizes a string for use in a Kubernetes resource name: lowercases,
/// replaces non-alphanumeric chars (except `-`) with `-`, optionally truncates
/// to `max_len`.
pub(crate) fn sanitize_k8s_name(input: &str, max_len: usize) -> String {
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

/// Sanitizes `route_key` and truncates the combined name to fit K8s limits.
///
/// User-supplied and possibly multi-byte, so it's run through
/// [`sanitize_k8s_name`] first; truncated char-safely to 63 chars (a byte
/// slice would panic mid-character) with any trailing `-` trimmed.
pub(crate) fn generate_ingress_route_name(service_name: &str, route_key: &str) -> String {
    let sanitized_key = sanitize_k8s_name(route_key, 63);
    let combined = format!("{service_name}-{sanitized_key}");
    let truncated: String = combined.chars().take(63).collect();
    truncated.trim_end_matches('-').to_string()
}

/// Generate the deterministic per-task preview `NetworkPolicy` name.
///
/// Format: `moatless-ws-{sanitized_task_id}-preview-ingress` (max 63 chars);
/// the task id is truncated to fit the fixed prefix/suffix, so the name never
/// ends in `-`.
pub fn generate_preview_network_policy_name(task_id: &str) -> String {
    let prefix = "moatless-ws-";
    let suffix = "-preview-ingress";
    let max_id_len = 63 - prefix.len() - suffix.len();
    let sanitized = sanitize_k8s_name(task_id, max_id_len);
    truncate_k8s(format!("{prefix}{sanitized}{suffix}"))
}

/// Generate a unique PVC name for a task.
///
/// Format: `moatless-sandbox-{sanitized_task_id}-{base36_timestamp}` (max 63
/// chars).
pub fn generate_pvc_name(task_id: &str) -> String {
    let sanitized = sanitize_k8s_name(task_id, 30);
    let timestamp = u128::try_from(chrono::Utc::now().timestamp_millis()).unwrap_or(0);
    let ts_str = radix_fmt(timestamp);
    truncate_k8s(format!("moatless-sandbox-{sanitized}-{ts_str}"))
}

/// Generate a unique pod name for a task.
///
/// Format: `moatless-{sanitized_repo}-{sanitized_task}-{hex_timestamp}` (max 63
/// chars).
pub fn generate_pod_name(repository_name: &str, task_id: &str) -> String {
    let sanitized_repo = sanitize_k8s_name(repository_name, 20);
    let sanitized_task = sanitize_k8s_name(task_id, 20);
    let timestamp = u128::try_from(chrono::Utc::now().timestamp_millis()).unwrap_or(0);
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
        assert_eq!(
            sanitize_k8s_name("a".repeat(100).as_str(), 20),
            "a".repeat(20)
        );
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
    fn test_generate_ingress_route_name_basic() {
        let svc = generate_service_name("task-123");
        assert_eq!(
            generate_ingress_route_name(&svc, "preview"),
            "moatless-ws-task-123-preview"
        );
        // User-supplied names are sanitized (uppercase/underscore → dash).
        assert_eq!(
            generate_ingress_route_name(&svc, "My_Server"),
            "moatless-ws-task-123-my-server"
        );
    }

    #[test]
    fn test_generate_ingress_route_name_multibyte_is_char_safe() {
        // A multi-byte UTF-8 name must not panic and must yield a valid,
        // ASCII-only name (a byte slice at 63 would have panicked here).
        let svc = generate_service_name("t");
        let name = generate_ingress_route_name(&svc, &"é".repeat(80));
        assert!(name.len() <= 63);
        assert!(name.is_ascii());
        assert!(!name.ends_with('-'));
    }

    #[test]
    fn test_generate_ingress_route_name_trims_trailing_dash() {
        // Truncation landing on a '-' (or a trailing sanitized '-') is trimmed —
        // a K8s name may not end in '-'.
        let svc = generate_service_name("task");
        assert!(!generate_ingress_route_name(&svc, "server-").ends_with('-'));
        let long = generate_ingress_route_name(&"a".repeat(62), "-x");
        assert!(long.len() <= 63);
        assert!(!long.ends_with('-'));
    }

    #[test]
    fn test_generate_ingress_route_name_truncates_to_63() {
        let svc = generate_service_name(&"a".repeat(60));
        let name = generate_ingress_route_name(&svc, &"b".repeat(60));
        assert!(name.len() <= 63);
    }

    #[test]
    fn test_generate_preview_network_policy_name() {
        assert_eq!(
            generate_preview_network_policy_name("abc123"),
            "moatless-ws-abc123-preview-ingress"
        );
        assert_eq!(
            generate_preview_network_policy_name("Task_ID"),
            "moatless-ws-task-id-preview-ingress"
        );
        let long = generate_preview_network_policy_name(&"a".repeat(100));
        assert!(long.len() <= 63);
        assert!(!long.ends_with('-'));
        assert!(long.ends_with("-preview-ingress"));
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
